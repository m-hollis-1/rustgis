//! The Esri enterprise geodatabase a SQL Server database may hold: finding its owner (`sde` or
//! `dbo`), reading its system tables into a [`Catalog`], and deciding how a registered class is
//! read (its base table, its versioned view, or its branch-versioned Default rows).
//!
//! The catalog rows are hostile input like any other server answer: every list is bounded by
//! [`MAX_ITEMS`] or `LIST_LIMIT`, and the owner is never taken from the server or the user as SQL:
//! only the two names [`owner_prefix`] knows are written into a statement.

use super::{Conn, LIST_LIMIT, Table, at, ident, int_of, text_of};
use crate::raster::Crs;
use crate::source::geodatabase::{
    self, BRANCH_DEFAULT_FILTER, Catalog, Item, ItemRow, LayerRow, MAX_ITEMS, ReadPlan, RegistryRow, Srs, Versioning,
};
use crate::{DataError, Result};

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
    Some(Queries {
        items: format!(
            "SELECT TOP ({MAX_ITEMS}) i.Name, i.PhysicalName, i.Path, t.Name, CAST(i.Definition AS nvarchar(max)) FROM {o}.GDB_ITEMS i JOIN {o}.GDB_ITEMTYPES t ON t.UUID = i.Type WHERE t.Name IN ('Feature Dataset', 'Feature Class', 'Table')"
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
    /// The FROM target: the base table or the versioned view, quoted.
    pub from: String,
    /// A predicate every read adds (the branch-versioned Default rows).
    pub filter: Option<String>,
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
    let native_spatial = matches!(table.kind.as_str(), "geometry" | "geography")
        && item.shape_column.as_deref().is_none_or(|s| s.eq_ignore_ascii_case(&table.column));
    // Only a traditionally versioned class is read through a view.
    let views: Vec<String> = if item.versioning == Versioning::Traditional {
        conn.query(
            &format!("SELECT TOP ({LIST_LIMIT}) TABLE_NAME FROM INFORMATION_SCHEMA.VIEWS WHERE TABLE_SCHEMA = @P1"),
            &[&table.schema],
        )?
        .iter()
        .filter_map(|r| text_of(at(r, 0)))
        .collect()
    } else {
        Vec::new()
    };
    let plan = geodatabase::read_plan(item, native_spatial, &views, columns);
    let srs = item.srid.and_then(|s| catalog.srs(s));
    reading_of(plan, table, item, srs).map(Some)
}

/// The reading a plan gives, worked out without a server.
pub(super) fn reading_of(plan: ReadPlan, table: &Table, item: &Item, srs: Option<&Srs>) -> Result<Reading> {
    let mut notes = Vec::new();
    let (from, filter) = match plan {
        ReadPlan::Base => (table.sql_from(), None),
        ReadPlan::VersionedView(view) => {
            notes.push(format!("versioned: read through its view {view}, which shows the Default version"));
            (format!("{}.{}", ident(&table.schema), ident(&view)), None)
        }
        ReadPlan::Branch => {
            notes.push("branch versioned: the Default version's rows are read".into());
            // The predicate is geodatabase::BRANCH_DEFAULT_FILTER, to be confirmed against Esri's
            // branch-versioning documentation (Task 17 desktop verification).
            (table.sql_from(), Some(BRANCH_DEFAULT_FILTER.to_string()))
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
        filter,
        rowid: item.rowid_column.clone(),
        alias: item.alias.clone(),
        field_aliases: item.field_aliases.clone(),
        srid_fallback,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::geodatabase::ItemKind;

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
        assert!(queries("sde").is_some_and(|q| q.items.contains("sde.GDB_ITEMS")));
        assert!(queries("x]y").is_none());
        assert_eq!(owner_prefix("sde"), Some("sde"));
        assert_eq!(owner_prefix("dbo"), Some("dbo"));
        for other in ["x]y", "SDE", "sde;", "", "dbo.x", "gis"] {
            assert_eq!(owner_prefix(other), None, "{other}");
        }
    }

    #[test]
    fn readings_map_plans_to_sql() {
        let (t, i) = (table(), item());
        let r = reading_of(ReadPlan::VersionedView("MAINS_evw".into()), &t, &i, None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((r.from.as_str(), r.filter.as_deref()), ("[SDE].[MAINS_evw]", None));
        assert_eq!(r.rowid.as_deref(), Some("OBJECTID"));
        assert_eq!(r.alias.as_deref(), Some("Water Mains"));
        assert_eq!(r.field_aliases, vec![("DIAMETER".to_string(), "Diameter (in)".to_string())]);
        assert_eq!(r.srid_fallback, None);

        let r = reading_of(ReadPlan::Branch, &t, &i, None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((r.from.as_str(), r.filter.as_deref()), ("[SDE].[MAINS]", Some(BRANCH_DEFAULT_FILTER)));

        let r = reading_of(ReadPlan::Base, &t, &i, None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((r.from.as_str(), r.filter.as_deref()), ("[SDE].[MAINS]", None));

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
}
