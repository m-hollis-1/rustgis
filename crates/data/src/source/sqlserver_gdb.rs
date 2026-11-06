//! The Esri enterprise geodatabase a SQL Server database may hold: finding its owner (`sde` or
//! `dbo`), reading its system tables into a [`Catalog`], and deciding how a registered class is
//! read (its base table, its versioned view, or its branch-versioned Default rows). A reading's
//! FROM target may be a derived table, so every statement gives it an alias (`FROM <from> t`).
//!
//! The catalog rows are hostile input like any other server answer: every list is bounded by
//! [`MAX_ITEMS`] or a small `TOP`, and the owner is never taken from the server or the user as SQL:
//! only the two names [`owner_prefix`] knows are written into a statement.

use super::{Conn, PROBE_TIMEOUT, Table, at, attr_of, field_cast, first_count, id_of, ident, int_of, text_of};
use crate::raster::Crs;
use crate::source::geodatabase::{
    self, Catalog, Item, ItemKind, ItemRow, LayerRow, MAX_DEFINITION_BYTES, MAX_ITEMS, ReadPlan, RegistryRow, Srs, is_machinery,
};
use crate::source::{Family, clamp_limit};
use crate::{DataError, Dataset, Feature, Features, Field, GeometryType, ReadReport, Result};

/// The owner as it may be written into a statement: only `sde` and `dbo` (what [`detect_owner`]
/// returns), never anything else.
pub(super) fn owner_prefix(owner: &str) -> Option<&'static str> {
    match owner {
        "sde" => Some("sde"),
        "dbo" => Some("dbo"),
        _ => None,
    }
}

/// The statements that read a geodatabase's catalog.
pub(super) struct Queries {
    pub items: String,
    pub registry: String,
    pub layers: String,
    pub srs: String,
    pub release: String,
}

/// The catalog statements for an owner; `None` for an owner [`owner_prefix`] refuses.
pub(super) fn queries(owner: &str) -> Option<Queries> {
    let o = owner_prefix(owner)?;
    // An over-size definition is left on the server (nvarchar is two bytes a character).
    let cap = 2 * MAX_DEFINITION_BYTES;
    Some(Queries {
        items: format!(
            "SELECT TOP ({MAX_ITEMS}) i.Name, i.PhysicalName, i.Path, t.Name, CASE WHEN DATALENGTH(i.Definition) > {cap} THEN NULL ELSE CAST(i.Definition AS nvarchar(max)) END FROM {o}.GDB_ITEMS i JOIN {o}.GDB_ITEMTYPES t ON t.UUID = i.Type WHERE t.Name IN ('Feature Dataset', 'Feature Class', 'Table')"
        ),
        registry: format!("SELECT TOP ({MAX_ITEMS}) owner, table_name, rowid_column FROM {o}.SDE_table_registry"),
        layers: format!("SELECT TOP ({MAX_ITEMS}) owner, table_name, spatial_column, srid FROM {o}.SDE_layers"),
        srs: format!("SELECT TOP ({MAX_ITEMS}) srid, auth_srid, srtext FROM {o}.SDE_spatial_references"),
        release: format!("SELECT TOP (1) major, minor, bugfix FROM {o}.SDE_version"),
    })
}

/// The geodatabase's owner: `sde` when `sde.GDB_ITEMS` exists, else `dbo` when `dbo.GDB_ITEMS`
/// does; `None` when the database holds no geodatabase.
pub(super) fn detect_owner(conn: &Conn) -> Result<Option<String>> {
    let rows = conn.query(
        "SELECT CASE WHEN OBJECT_ID('sde.GDB_ITEMS') IS NOT NULL THEN 'sde' WHEN OBJECT_ID('dbo.GDB_ITEMS') IS NOT NULL THEN 'dbo' END",
        &[],
    )?;
    Ok(rows.first().and_then(|r| text_of(at(r, 0))).filter(|o| owner_prefix(o).is_some()))
}

/// Reads the catalog of the geodatabase `owner` holds. A missing or unreadable `SDE_version`
/// leaves the release unknown; any other failure is the error.
pub(super) fn read_catalog(conn: &Conn, owner: &str) -> Result<Catalog> {
    let q = queries(owner).ok_or_else(|| DataError::Format(format!("'{owner}' isn't a geodatabase owner")))?;
    let items = conn
        .query(&q.items, &[])?
        .iter()
        .filter_map(|r| {
            Some(ItemRow {
                name: text_of(at(r, 0))?,
                physical: text_of(at(r, 1)).unwrap_or_default(),
                path: text_of(at(r, 2)).unwrap_or_default(),
                type_name: text_of(at(r, 3))?,
                definition: text_of(at(r, 4)),
            })
        })
        .collect();
    let registry = conn
        .query(&q.registry, &[])?
        .iter()
        .filter_map(|r| Some(RegistryRow { owner: text_of(at(r, 0))?, table: text_of(at(r, 1))?, rowid_column: text_of(at(r, 2)) }))
        .collect();
    let layers = conn
        .query(&q.layers, &[])?
        .iter()
        .filter_map(|r| {
            Some(LayerRow { owner: text_of(at(r, 0))?, table: text_of(at(r, 1))?, column: text_of(at(r, 2))?, srid: int_of(at(r, 3)) })
        })
        .collect();
    let srs = conn
        .query(&q.srs, &[])?
        .iter()
        .filter_map(|r| Some(Srs { srid: int_of(at(r, 0))?, auth_srid: int_of(at(r, 1)), srtext: text_of(at(r, 2)) }))
        .collect();
    let release = conn.query(&q.release, &[]).ok().and_then(|rows| {
        let r = rows.first()?;
        Some(format!("{}.{}.{}", int_of(at(r, 0))?, int_of(at(r, 1))?, int_of(at(r, 2))?))
    });
    Ok(Catalog::from_rows(owner, release, items, registry, layers, srs))
}

/// How a registered class is read.
#[derive(Debug)]
pub(super) struct Reading {
    /// The FROM target, without an alias: the base table or the versioned view, quoted, or the
    /// derived table of a branch-versioned class's Default rows.
    pub from: String,
    /// The registered ObjectID column.
    pub rowid: Option<String>,
    /// The class alias.
    pub alias: Option<String>,
    /// `(field name, alias)`.
    pub field_aliases: Vec<(String, String)>,
    /// The EPSG code of the class's spatial reference, for a column whose rows say SRID 0.
    pub srid_fallback: Option<u32>,
    pub notes: Vec<String>,
}

/// How to read `table` when the catalog registers it; Ok(None) for an unregistered table (or a
/// database without a geodatabase). `columns` are the base table's column names.
pub(super) fn reading(conn: &Conn, table: &Table, columns: &[String]) -> Result<Option<Reading>> {
    let Some(catalog) = super::geodatabase(conn) else { return Ok(None) };
    let Some(item) = catalog.find(&format!("{}.{}", table.schema, table.name)) else { return Ok(None) };
    let native_spatial = native_spatial(table);
    let views = views_of(conn, item, &table.schema)?;
    let plan = geodatabase::read_plan(item, native_spatial, &views, columns);
    let srs = item.srid.and_then(|s| catalog.srs(s));
    reading_of(plan, table, item, srs).map(Some)
}

/// The versioned views of a registered `item` that exist in `schema` (`<table>_evw` and
/// `<table>_vw`), asked for whatever its definition says, so a class whose definition calls it
/// unversioned is still read through the view it has.
fn views_of(conn: &Conn, item: &Item, schema: &str) -> Result<Vec<String>> {
    let (evw, vw) = (format!("{}_evw", item.table), format!("{}_vw", item.table));
    let rows = conn.query(
        "SELECT TOP (2) TABLE_NAME FROM INFORMATION_SCHEMA.VIEWS WHERE TABLE_SCHEMA = @P1 AND TABLE_NAME IN (@P2, @P3)",
        &[schema, &evw, &vw],
    )?;
    Ok(rows.iter().filter_map(|r| text_of(at(r, 0))).collect())
}

/// What a geometry-kind probe of a registered class needs: the kind its definition gives, or else
/// the FROM target its rows are read through.
pub(super) enum Probe {
    Known(Family),
    From(String),
}

/// How the kinds of geometry of `table` are found when the catalog registers it; Ok(None) for an
/// unregistered table (probed as it is). A class whose definition names its shape type is that
/// kind; any other is probed through its reading (its versioned view or Default rows), not its
/// base table, whose rows may still be in the delta tables.
pub(super) fn probe(conn: &Conn, table: &Table) -> Result<Option<Probe>> {
    let Some(catalog) = super::geodatabase(conn) else { return Ok(None) };
    let Some(item) = catalog.find(&format!("{}.{}", table.schema, table.name)) else { return Ok(None) };
    if let Some(family) = item.family {
        return Ok(Some(Probe::Known(family)));
    }
    let names: Vec<String> = columns_of(conn, &table.schema, &table.name)?.into_iter().map(|(n, _)| n).collect();
    let views = views_of(conn, item, &table.schema)?;
    let plan = geodatabase::read_plan(item, native_spatial(table), &views, &names);
    Ok(Some(Probe::From(reading_of(plan, table, item, None)?.from)))
}

/// The derived table of a branch-versioned class's Default rows: the latest row per ObjectID on
/// branch 0 (Default) that isn't a delete. Each edit inserts a row; the older rows of the same
/// ObjectID aren't marked, so the latest is picked by `GDB_FROM_DATE`. Task 17 (desktop
/// verification) confirms the columns against a real branch-versioned class.
fn branch_default_from(base: &str, rowid: &str) -> String {
    format!(
        "(SELECT * FROM (SELECT b.*, ROW_NUMBER() OVER (PARTITION BY b.{} ORDER BY b.GDB_FROM_DATE DESC) AS rustgis_rn FROM {base} b WHERE b.GDB_BRANCH_ID = 0) r WHERE r.rustgis_rn = 1 AND r.GDB_IS_DELETE = 0)",
        ident(rowid)
    )
}

/// The `(name, lower-case data type)` of every column of a table or view, in table order.
pub(super) fn columns_of(conn: &Conn, schema: &str, table: &str) -> Result<Vec<(String, String)>> {
    let rows = conn.query(
        "SELECT COLUMN_NAME, LOWER(DATA_TYPE) FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_SCHEMA = @P1 AND TABLE_NAME = @P2 ORDER BY ORDINAL_POSITION",
        &[schema, table],
    )?;
    Ok(rows.iter().filter_map(|r| Some((text_of(at(r, 0))?, text_of(at(r, 1))?))).collect())
}

/// Whether the column is a native spatial column. Any of a class's `geometry` or `geography`
/// columns (its shape or another) reads through the class's plan: the versioned view carries
/// every column. Only `tables` columns reach here, so this is false only for other storage.
pub(super) fn native_spatial(table: &Table) -> bool {
    matches!(table.kind.as_str(), "geometry" | "geography")
}

/// The note for a table read without the catalog because the catalog couldn't be read. The
/// problem text comes from `Conn::query`'s errors, which are redacted of the password.
pub(super) fn catalog_note(problem: Option<&str>) -> Option<String> {
    problem.map(|why| {
        format!(
            "the geodatabase catalog couldn't be read ({why}): a versioned table shows its base rows, without the edits in its delta tables"
        )
    })
}

/// The reading a plan gives, worked out without a server.
pub(super) fn reading_of(plan: ReadPlan, table: &Table, item: &Item, srs: Option<&Srs>) -> Result<Reading> {
    let mut notes = Vec::new();
    let from = match plan {
        ReadPlan::Base => table.sql_from(),
        ReadPlan::VersionedView(view) => {
            notes.push(format!("versioned: read through its view {view}, which shows the Default version"));
            format!("{}.{}", ident(&table.schema), ident(&view))
        }
        ReadPlan::Branch(rowid) => {
            notes.push("branch versioned: the Default version's rows are read".into());
            branch_default_from(&table.sql_from(), &rowid)
        }
        ReadPlan::Refused(why) => return Err(DataError::Unsupported(why)),
    };
    let srid_fallback = srs.and_then(|s| {
        s.auth_srid
            .filter(|a| *a > 0)
            .and_then(|a| u32::try_from(a).ok())
            .or_else(|| s.srtext.as_deref().and_then(|t| Crs::parse(t).ok()).map(|c| c.epsg()).filter(|e| *e > 0).map(u32::from))
    });
    Ok(Reading {
        from,
        rowid: item.rowid_column.clone(),
        alias: item.alias.clone(),
        field_aliases: item.field_aliases.clone(),
        srid_fallback,
        notes,
    })
}

/// What a geodatabase lists: its feature datasets, its registered feature classes and tables, and
/// the spatial tables it doesn't register that aren't its own machinery.
#[derive(Clone, Debug, PartialEq)]
pub struct GdbListing {
    /// The feature datasets' short names, in catalog order.
    pub datasets: Vec<String>,
    /// Every registered feature class with its spatial column (carrying the class alias and
    /// dataset), or `None` when the table holds no native spatial column: it is listed, and
    /// refuses at open.
    pub classes: Vec<(Item, Option<Table>)>,
    /// The registered non-spatial tables.
    pub tables: Vec<Item>,
    /// The spatial tables the catalog doesn't register, without the geodatabase's machinery.
    pub other: Vec<Table>,
}

/// Groups the catalog's items with the database's spatial columns, worked out without a server.
pub(super) fn listing_of(catalog: &Catalog, spatial: Vec<Table>) -> GdbListing {
    let same = |t: &Table, i: &Item| t.schema.eq_ignore_ascii_case(&i.schema) && t.name.eq_ignore_ascii_case(&i.table);
    let data = || catalog.items.iter().filter(|i| i.kind != ItemKind::FeatureDataset);
    let classes = data()
        .filter(|i| i.kind == ItemKind::FeatureClass)
        .map(|item| {
            let shape = item.shape_column.as_deref();
            let table = spatial
                .iter()
                .find(|t| same(t, item) && shape.is_some_and(|s| t.column.eq_ignore_ascii_case(s)))
                .or_else(|| spatial.iter().find(|t| same(t, item)))
                .map(|t| Table {
                    alias: item.alias.clone(),
                    dataset: item.dataset.clone(),
                    short: Some(item.short_name().to_string()),
                    ..t.clone()
                });
            (item.clone(), table)
        })
        .collect();
    let tables = data().filter(|i| i.kind == ItemKind::Table).cloned().collect();
    let other = spatial.iter().filter(|t| !is_machinery(&t.name) && !data().any(|i| same(t, i))).cloned().collect();
    let datasets = catalog.feature_datasets().iter().map(|d| d.short_name().to_string()).collect();
    GdbListing { datasets, classes, tables, other }
}

/// The geodatabase's listing; `None` when the database holds none (or its catalog can't be read:
/// see [`super::geodatabase_problem`]).
pub fn geodatabase_tables(conn: &Conn) -> Result<Option<GdbListing>> {
    let Some(catalog) = super::geodatabase(conn) else { return Ok(None) };
    Ok(Some(listing_of(&catalog, super::tables(conn)?)))
}

/// The statement that reads a registered table's rows, and the fields it yields. The first
/// column is the feature id: the registered ObjectID column when the table has it as an integer
/// (the rows are then read in its order), else the row number. The rest follow the fields, each
/// cast as [`field_cast`] does; geometry and binary columns are left out.
fn table_select(reading: &Reading, columns: &[(String, String)], limit: usize) -> (Vec<Field>, String) {
    let rowid = reading.rowid.as_deref().and_then(|want| {
        columns
            .iter()
            .find(|(name, ty)| name.eq_ignore_ascii_case(want) && matches!(ty.as_str(), "tinyint" | "smallint" | "int" | "bigint"))
            .map(|(name, _)| ident(name))
    });
    let mut fields = Vec::new();
    let mut select = vec![match &rowid {
        Some(id) => format!("CAST({id} AS BIGINT)"),
        None => "ROW_NUMBER() OVER (ORDER BY (SELECT NULL))".to_string(),
    }];
    for (name, ty) in columns {
        let Some((field_type, cast)) = field_cast(ty, &ident(name)) else { continue };
        let mut field = Field::new(name.clone(), field_type);
        field.alias = reading.field_aliases.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, alias)| alias.clone());
        fields.push(field);
        select.push(cast);
    }
    let mut sql = format!("SELECT TOP ({limit}) {} FROM {} t", select.join(", "), reading.from);
    if let Some(id) = rowid {
        sql.push_str(&format!(" ORDER BY {id}"));
    }
    (fields, sql)
}

/// A registered non-spatial table loaded whole (through its versioned view when versioned), at
/// most `limit` rows, its fields carrying their geodatabase aliases. The report says when there
/// are more rows than were read.
pub fn read_table(conn: &Conn, item: &Item, limit: usize) -> Result<(Dataset, ReadReport)> {
    let limit = clamp_limit(limit);
    let columns = columns_of(conn, &item.schema, &item.table)?;
    let names: Vec<String> = columns.iter().map(|(n, _)| n.clone()).collect();
    let views = views_of(conn, item, &item.schema)?;
    // A table has no storage question, so the plan is asked as for a native column.
    let plan = geodatabase::read_plan(item, true, &views, &names);
    let base = Table {
        schema: item.schema.clone(),
        name: item.table.clone(),
        column: String::new(),
        kind: String::new(),
        current: false,
        several: false,
        alias: None,
        dataset: None,
        short: None,
    };
    let reading = reading_of(plan, &base, item, None)?;
    let (fields, sql) = table_select(&reading, &columns, limit);
    let rows = conn.query(&sql, &[])?;
    let features: Features = rows
        .iter()
        .zip(1u64..)
        .map(|(r, n)| Feature {
            id: id_of(at(r, 0)).unwrap_or(n),
            geometry: None,
            attrs: fields.iter().enumerate().map(|(i, f)| attr_of(f.ty, at(r, i + 1))).collect(),
        })
        .collect();
    let mut report = ReadReport { skipped: 0, messages: reading.notes.clone() };
    let count_sql = format!("SELECT COUNT_BIG(*) FROM {} t", reading.from);
    let count = conn.query_within(&count_sql, &[], PROBE_TIMEOUT).ok().flatten().and_then(|r| first_count(&r));
    if let Some(count) = count.filter(|c| *c > limit as u64) {
        report.messages.push(format!("the first {limit} rows of {count}"));
    }
    let mut dataset = Dataset::new(reading.alias.clone().unwrap_or_else(|| item.short_name().to_string()), GeometryType::None, fields);
    dataset.features = features;
    dataset.source = Some(format!("{}.{}", item.schema, item.table));
    Ok((dataset, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::geodatabase::{ItemKind, Versioning};

    const WGS84: &str = r#"GEOGCS["GCS_WGS_1984",DATUM["D_WGS_1984",SPHEROID["WGS_1984",6378137.0,298.257223563]],PRIMEM["Greenwich",0.0],UNIT["Degree",0.0174532925199433]]"#;

    fn table() -> Table {
        Table {
            schema: "SDE".into(),
            name: "MAINS".into(),
            column: "SHAPE".into(),
            kind: "geometry".into(),
            current: false,
            several: false,
            alias: None,
            dataset: None,
            short: None,
        }
    }

    fn item() -> Item {
        Item {
            name: "utility.sde.Mains".into(),
            schema: "SDE".into(),
            table: "MAINS".into(),
            kind: ItemKind::FeatureClass,
            dataset: Some("Water".into()),
            alias: Some("Water Mains".into()),
            shape_column: Some("SHAPE".into()),
            field_aliases: vec![("DIAMETER".into(), "Diameter (in)".into())],
            versioning: Versioning::Traditional,
            definition_ok: true,
            family: Some(Family::Lines),
            archived: false,
            rowid_column: Some("OBJECTID".into()),
            srid: Some(3),
        }
    }

    #[test]
    fn catalog_sql_names_the_owner() {
        let q = queries("dbo").unwrap_or_else(|| panic!("dbo is an owner"));
        for (sql, table) in [
            (&q.items, "dbo.GDB_ITEMS"),
            (&q.registry, "dbo.SDE_table_registry"),
            (&q.layers, "dbo.SDE_layers"),
            (&q.srs, "dbo.SDE_spatial_references"),
            (&q.release, "dbo.SDE_version"),
        ] {
            assert!(sql.contains(table), "{sql}");
        }
        for sql in [&q.items, &q.registry, &q.layers, &q.srs] {
            assert!(sql.contains("TOP (10000)"), "{sql}");
        }
        assert!(q.items.contains("dbo.GDB_ITEMTYPES"), "{}", q.items);
        assert!(q.release.contains("TOP (1)"), "{}", q.release);
        assert!(
            q.items.contains(&format!(
                "CASE WHEN DATALENGTH(i.Definition) > {} THEN NULL ELSE CAST(i.Definition AS nvarchar(max)) END",
                2 * MAX_DEFINITION_BYTES
            )),
            "{}",
            q.items
        );
        assert!(queries("sde").is_some_and(|q| q.items.contains("sde.GDB_ITEMS")));
        assert!(queries("x]y").is_none());
        assert_eq!(owner_prefix("sde"), Some("sde"));
        assert_eq!(owner_prefix("dbo"), Some("dbo"));
        for other in ["x]y", "SDE", "sde;", "", "dbo.x", "gis"] {
            assert_eq!(owner_prefix(other), None, "{other}");
        }
    }

    #[test]
    fn any_native_spatial_column_reads_through_the_plan() {
        let second = Table { column: "SHAPE2".into(), ..table() };
        assert!(native_spatial(&second));
        assert!(!native_spatial(&Table { kind: "varbinary".into(), ..table() }));
        let views = ["MAINS_evw".to_string()];
        let unversioned = Item { versioning: Versioning::Unversioned, ..item() };
        assert_eq!(geodatabase::read_plan(&unversioned, native_spatial(&second), &[], &[]), ReadPlan::Base);
        assert_eq!(geodatabase::read_plan(&item(), native_spatial(&second), &views, &[]), ReadPlan::VersionedView("MAINS_evw".into()));
    }

    #[test]
    fn an_unreadable_catalog_is_noted() {
        assert_eq!(catalog_note(None), None);
        assert_eq!(
            catalog_note(Some("permission denied")).as_deref(),
            Some(
                "the geodatabase catalog couldn't be read (permission denied): a versioned table shows its base rows, without the edits in its delta tables"
            )
        );
    }

    #[test]
    fn readings_map_plans_to_sql() {
        let (t, i) = (table(), item());
        let r = reading_of(ReadPlan::VersionedView("MAINS_evw".into()), &t, &i, None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r.from, "[SDE].[MAINS_evw]");
        assert_eq!(r.rowid.as_deref(), Some("OBJECTID"));
        assert_eq!(r.alias.as_deref(), Some("Water Mains"));
        assert_eq!(r.field_aliases, vec![("DIAMETER".to_string(), "Diameter (in)".to_string())]);
        assert_eq!(r.srid_fallback, None);

        let r = reading_of(ReadPlan::Branch("OBJECTID".into()), &t, &i, None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            r.from,
            "(SELECT * FROM (SELECT b.*, ROW_NUMBER() OVER (PARTITION BY b.[OBJECTID] ORDER BY b.GDB_FROM_DATE DESC) AS rustgis_rn FROM [SDE].[MAINS] b WHERE b.GDB_BRANCH_ID = 0) r WHERE r.rustgis_rn = 1 AND r.GDB_IS_DELETE = 0)"
        );
        assert!(!r.from.contains("GDB_TO_DATE"), "{}", r.from);

        let r = reading_of(ReadPlan::Base, &t, &i, None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r.from, "[SDE].[MAINS]");

        let refused = reading_of(ReadPlan::Refused("not readable".into()), &t, &i, None);
        assert!(matches!(refused, Err(DataError::Unsupported(ref why)) if why == "not readable"), "{refused:?}");

        let srs = Srs { srid: 3, auth_srid: Some(2263), srtext: Some(WGS84.into()) };
        let r = reading_of(ReadPlan::Base, &t, &i, Some(&srs)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r.srid_fallback, Some(2263));

        let expected = Crs::parse(WGS84).map(|c| c.epsg()).unwrap_or(0);
        assert!(expected > 0);
        for auth_srid in [None, Some(0)] {
            let srs = Srs { srid: 3, auth_srid, srtext: Some(WGS84.into()) };
            let r = reading_of(ReadPlan::Base, &t, &i, Some(&srs)).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(r.srid_fallback, Some(u32::from(expected)));
        }
        let srs = Srs { srid: 3, auth_srid: None, srtext: Some("not a coordinate system".into()) };
        let r = reading_of(ReadPlan::Base, &t, &i, Some(&srs)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r.srid_fallback, None);
    }

    fn spatial(schema: &str, name: &str, column: &str) -> Table {
        Table { schema: schema.into(), name: name.into(), column: column.into(), ..table() }
    }

    fn catalog() -> Catalog {
        use geodatabase::{ItemRow, LayerRow, RegistryRow};
        let row = |name: &str, physical: &str, path: &str, type_name: &str| ItemRow {
            name: name.into(),
            physical: physical.into(),
            path: path.into(),
            type_name: type_name.into(),
            definition: None,
        };
        Catalog::from_rows(
            "sde",
            None,
            vec![
                row("utility.sde.Water", "", r"\Water", "Feature Dataset"),
                row("utility.sde.Mains", "SDE.MAINS", r"\Water\Mains", "Feature Class"),
                row("sde.Readings", "SDE.READINGS", r"\Readings", "Table"),
            ],
            vec![RegistryRow { owner: "SDE".into(), table: "MAINS".into(), rowid_column: Some("OBJECTID".into()) }],
            vec![LayerRow { owner: "SDE".into(), table: "MAINS".into(), column: "SHAPE".into(), srid: Some(2) }],
            vec![],
        )
    }

    #[test]
    fn listing_groups_items_and_hides_machinery() {
        let catalog = catalog();
        let listing = listing_of(
            &catalog,
            vec![
                spatial("SDE", "MAINS", "SHAPE"),
                spatial("SDE", "MAINS_evw", "SHAPE"),
                spatial("SDE", "A12", "SHAPE"),
                spatial("dbo", "Parcels", "geom"),
            ],
        );
        assert_eq!(listing.datasets, ["Water"]);
        let mains = catalog.find("Mains").unwrap_or_else(|| panic!("Mains is registered"));
        let expected = Table { dataset: Some("Water".into()), short: Some("Mains".into()), ..spatial("SDE", "MAINS", "SHAPE") };
        assert_eq!(listing.classes, vec![(mains.clone(), Some(expected))]);
        let readings = catalog.find("Readings").unwrap_or_else(|| panic!("Readings is registered"));
        assert_eq!(listing.tables, vec![readings.clone()]);
        assert_eq!(listing.other, vec![spatial("dbo", "Parcels", "geom")]);
    }

    #[test]
    fn a_class_lists_its_shape_column_with_alias_and_dataset() {
        let mut catalog = catalog();
        if let Some(mains) = catalog.items.iter_mut().find(|i| i.table == "MAINS") {
            mains.alias = Some("Water Mains".into());
        }
        let listing = listing_of(&catalog, vec![spatial("sde", "mains", "EXTRA"), spatial("sde", "mains", "SHAPE")]);
        let table = listing.classes.first().and_then(|(_, t)| t.as_ref()).unwrap_or_else(|| panic!("a spatial column"));
        assert_eq!(
            (table.column.as_str(), table.alias.as_deref(), table.dataset.as_deref()),
            ("SHAPE", Some("Water Mains"), Some("Water"))
        );
        assert!(listing.other.is_empty(), "{:?}", listing.other);
        // No native spatial column: still listed.
        let listing = listing_of(&catalog, vec![]);
        assert_eq!(listing.classes.len(), 1);
        assert!(listing.classes.iter().all(|(_, t)| t.is_none()));
        // Without the shape column named, the first spatial column is the class's.
        let mut unnamed = catalog.clone();
        if let Some(mains) = unnamed.items.iter_mut().find(|i| i.table == "MAINS") {
            mains.shape_column = None;
        }
        let listing = listing_of(&unnamed, vec![spatial("SDE", "MAINS", "EXTRA"), spatial("SDE", "MAINS", "SHAPE")]);
        assert_eq!(listing.classes.first().and_then(|(_, t)| t.as_ref()).map(|t| t.column.as_str()), Some("EXTRA"));
    }

    #[test]
    fn a_table_read_selects_the_columns_in_row_id_order() {
        let columns: Vec<(String, String)> =
            [("OBJECTID", "int"), ("DIAMETER", "float"), ("SHAPE", "geometry"), ("NOTE", "nvarchar"), ("RAW", "varbinary")]
                .iter()
                .map(|(n, t)| (n.to_string(), t.to_string()))
                .collect();
        let (t, i) = (table(), item());
        let reading = reading_of(ReadPlan::VersionedView("MAINS_evw".into()), &t, &i, None).unwrap_or_else(|e| panic!("{e}"));
        let (fields, sql) = table_select(&reading, &columns, 50);
        assert_eq!(
            sql,
            "SELECT TOP (50) CAST([OBJECTID] AS BIGINT), CAST([OBJECTID] AS BIGINT), CAST([DIAMETER] AS FLOAT), CAST([NOTE] AS NVARCHAR(MAX)) FROM [SDE].[MAINS_evw] t ORDER BY [OBJECTID]"
        );
        let names: Vec<(&str, Option<&str>)> = fields.iter().map(|f| (f.name.as_str(), f.alias.as_deref())).collect();
        assert_eq!(names, [("OBJECTID", None), ("DIAMETER", Some("Diameter (in)")), ("NOTE", None)]);

        // A branch table reads from its Default rows' derived table, aliased.
        let branch = reading_of(ReadPlan::Branch("OBJECTID".into()), &t, &i, None).unwrap_or_else(|e| panic!("{e}"));
        let (_, sql) = table_select(&branch, &columns, 5);
        assert!(sql.ends_with(&format!("FROM {} t ORDER BY [OBJECTID]", branch.from)), "{sql}");

        // Without a registered row id the rows are numbered.
        let unnumbered = Reading { rowid: None, ..reading_of(ReadPlan::Base, &t, &i, None).unwrap_or_else(|e| panic!("{e}")) };
        let (_, sql) = table_select(&unnumbered, &columns, 5);
        assert!(sql.starts_with("SELECT TOP (5) ROW_NUMBER() OVER (ORDER BY (SELECT NULL)), "), "{sql}");

        // A row id column the table lacks, or that isn't an integer, is no ordering.
        let missing = Reading { rowid: Some("GONE".into()), ..reading_of(ReadPlan::Base, &t, &i, None).unwrap_or_else(|e| panic!("{e}")) };
        assert!(table_select(&missing, &columns, 5).1.ends_with("FROM [SDE].[MAINS] t"));
        let text = Reading { rowid: Some("note".into()), ..missing };
        assert!(table_select(&text, &columns, 5).1.ends_with("FROM [SDE].[MAINS] t"));
    }
}
