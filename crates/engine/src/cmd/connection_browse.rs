//! `connection.browse`: the datasets a saved connection offers, one level at a time, each with
//! the command that adds it.
//!
//! The app reads over the network in the background, so browsing is a probe/answer pair like
//! `connection.test`: `probe` names the address a level reads, the app fetches it and asks again
//! with the `answer`. Without an answer, the command line and the assistant read it themselves.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use std::borrow::Cow;

use super::super::{b, ps};
use crate::connections::{Connection, DESKTOP_ONLY};
use crate::render::rustgis_tiles::{address, s3, stac};
use crate::{CmdResult, Session};

/// The most children of a non-database level; database levels use the provider's catalog.
const MAX_CHILDREN: usize = 500;
/// The items of a STAC collection that are listed.
const ITEMS: usize = 20;
/// The longest node a caller may give.
const MAX_NODE: usize = 4096;

/// What a node of a connection stands for.
enum Level {
    Database,
    DatabaseSchema {
        schema: String,
    },
    /// A geodatabase's feature dataset.
    DatabaseDataset {
        dataset: String,
    },
    Service {
        url: String,
        kind: Option<String>,
    },
    Catalogue,
    StacRoot {
        api: String,
    },
    StacCollection {
        api: String,
        id: String,
    },
    StacCatalog {
        url: String,
    },
    StacItem {
        url: String,
    },
    S3Root,
    S3Prefix {
        bucket: String,
        prefix: String,
    },
}

/// The types a `service:<type>:<url>` node names (`auto` leaves the type to be guessed).
const SERVICE_TYPES: [&str; 7] = ["auto", "wms", "wmts", "wfs", "wcs", "ogcapi", "arcgis"];

/// The level a connection's `node` names (its top level when empty).
fn level(c: &Connection, node: &str) -> Result<Level, String> {
    let kind = c.kind.as_str();
    let not_a_node = || format!("'{node}' isn't a node of {}", c.name);
    if node.is_empty() {
        return Ok(match kind {
            "postgres" | "mysql" | "sqlserver" => {
                if cfg!(target_arch = "wasm32") {
                    return Err(DESKTOP_ONLY.into());
                }
                Level::Database
            }
            "service" => Level::Service { url: c.address.clone(), kind: None },
            "wms" | "wmts" | "wfs" | "wcs" | "ogcapi" | "arcgis" => Level::Service { url: c.address.clone(), kind: Some(c.kind.clone()) },
            "csw" => Level::Catalogue,
            "stac" => Level::StacRoot { api: stac::api(Some(&c.address))? },
            "s3" => Level::S3Root,
            _ => return Err(format!("unknown kind '{}'", c.kind)),
        });
    }
    if let Some(rest) = node.strip_prefix("service:") {
        if !matches!(kind, "service" | "wms" | "wmts" | "wfs" | "wcs" | "ogcapi" | "arcgis" | "csw") {
            return Err(not_a_node());
        }
        let (t, url) = rest.split_once(':').filter(|(t, _)| SERVICE_TYPES.contains(t)).ok_or_else(not_a_node)?;
        // Judged as a document of the connection would be: a link may not lead into the local network.
        address::check(url, Some(&c.address))?;
        return Ok(Level::Service { url: url.to_string(), kind: (t != "auto").then(|| t.to_string()) });
    }
    // A schema's or a geodatabase dataset's name, as `schemas` and `geodatabase_tree` encode it.
    let named = |encoded: &str| {
        if !matches!(kind, "postgres" | "mysql" | "sqlserver") || encoded.is_empty() {
            return Err(not_a_node());
        }
        URL_SAFE_NO_PAD
            .decode(encoded)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .filter(|name| !name.is_empty())
            .ok_or_else(not_a_node)
    };
    if let Some(schema) = node.strip_prefix("schema:") {
        return Ok(Level::DatabaseSchema { schema: named(schema)? });
    }
    if let Some(dataset) = node.strip_prefix("dataset:") {
        if kind != "sqlserver" {
            return Err(not_a_node());
        }
        return Ok(Level::DatabaseDataset { dataset: named(dataset)? });
    }
    if let Some(id) = node.strip_prefix("collection:") {
        if kind != "stac" || id.is_empty() {
            return Err(not_a_node());
        }
        let api = stac::api(Some(&c.address))?;
        if stac::catalog::is_static(&api) {
            return Err(not_a_node());
        }
        return Ok(Level::StacCollection { api, id: id.to_string() });
    }
    if let Some(url) = node.strip_prefix("catalog:") {
        if kind != "stac" {
            return Err(not_a_node());
        }
        address::check(url, Some(&c.address))?;
        return Ok(Level::StacCatalog { url: url.to_string() });
    }
    if let Some(url) = node.strip_prefix("item:") {
        // A static catalog's item: its own file, read when opened.
        if kind != "stac" {
            return Err(not_a_node());
        }
        address::check(url, Some(&c.address))?;
        return Ok(Level::StacItem { url: url.to_string() });
    }
    if let Some(rest) = node.strip_prefix("s3:") {
        let (bucket, prefix) = rest
            .split_once('/')
            .filter(|(bucket, prefix)| kind == "s3" && s3::valid_bucket(bucket) && prefix.len() <= 1024)
            .ok_or_else(not_a_node)?;
        return Ok(Level::S3Prefix { bucket: bucket.to_string(), prefix: prefix.to_string() });
    }
    Err(not_a_node())
}

/// The session's S3 connection for an S3 level, logging in with what needs no typing (a profile,
/// the environment, anonymously, or the keys saved on this computer: browsing is the user action
/// that may ask the system to unlock its store).
/// (A probe logs in too, as `connection.test` does: the address it names, and the signature the
/// fetch needs, depend on the connection.)
fn s3_login(s: &mut Session, c: &Connection, level: &Level) -> Result<Option<s3::Connection>, String> {
    if !matches!(level, Level::S3Root | Level::S3Prefix { .. }) {
        return Ok(None);
    }
    super::ensure_login(s, c)?;
    let conn = s3::connection(&c.name).ok_or("log in first: connection.login {id, accessKeyId, secretAccessKey}")?;
    if let Level::S3Prefix { bucket, .. } = level {
        super::grant(s, c, &conn, bucket);
    }
    Ok(Some(conn))
}

/// The address a level reads (`None`: nothing is fetched, or the database driver reads it).
fn fetch_url(s: &Session, c: &Connection, level: &Level, conn: Option<&s3::Connection>) -> Result<Option<String>, String> {
    Ok(match level {
        Level::Database | Level::DatabaseSchema { .. } | Level::DatabaseDataset { .. } => None,
        Level::Service { url, kind } => Some(rustgis_data::ogc::capabilities_url(url, kind.as_deref())),
        Level::Catalogue => Some(super::super::service::records_request(s, &json!({"url": c.address}))?),
        Level::StacRoot { api } => Some(if stac::catalog::is_static(api) { api.clone() } else { stac::collections_url(api) }),
        Level::StacCollection { api, id } => {
            Some(stac::search_url(api, &stac::Search { collections: vec![id.clone()], limit: ITEMS, ..stac::Search::default() })?)
        }
        Level::StacCatalog { url } | Level::StacItem { url } => Some(url.clone()),
        Level::S3Root => match (super::first_bucket(c), conn) {
            (Some(_), _) => None,
            (None, Some(conn)) if conn.credentials.is_some() => Some(super::through(s, c, conn, s3::buckets_url(conn))?),
            _ => return Err("name a bucket under Buckets to browse an anonymous S3 connection".into()),
        },
        Level::S3Prefix { bucket, prefix } => {
            let url = s3::list_url(bucket, prefix, None, conn);
            Some(match conn {
                Some(conn) => super::through(s, c, conn, url)?,
                None => url,
            })
        }
    })
}

/// What a level reads, fetched here (waiting for it).
#[cfg(not(target_arch = "wasm32"))]
fn download(url: &str) -> Result<String, String> {
    let bytes = crate::render::rustgis_tiles::cache::download_blocking(url)?;
    Ok(String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()))
}

#[cfg(target_arch = "wasm32")]
fn download(_: &str) -> Result<String, String> {
    Err("in the web app, expand the connection under Data Sources › Connections".into())
}

/// The children of a level, and what is known of the rest.
#[derive(Default)]
struct Found {
    children: Vec<Value>,
    truncated: bool,
    matched: Option<u64>,
}

impl Found {
    /// Keeps the first [`MAX_CHILDREN`] of a list (collected with one more, to know it was cut).
    fn new(mut children: Vec<Value>) -> Found {
        let truncated = children.len() > MAX_CHILDREN;
        children.truncate(MAX_CHILDREN);
        Found { children, truncated, matched: None }
    }
}

/// A node that fits what [`browse`] accepts.
fn node_ok(node: &str) -> bool {
    node.len() <= MAX_NODE && !node.chars().any(char::is_control)
}

pub(super) fn browse(s: &mut Session, p: &Value) -> CmdResult {
    let c = super::found(s, p)?.clone();
    let node = ps(p, "node").unwrap_or("");
    if !node_ok(node) {
        return Err("node is a child's node, as connection.browse lists it".into());
    }
    // The app's answers come from a fetch the cache already limits; a caller's may be any size.
    let documents = p.get("documents").and_then(Value::as_array).into_iter().flatten().filter_map(|d| ps(d, "text")).map(str::len);
    if ps(p, "answer").map(str::len).into_iter().chain(documents).sum::<usize>() > super::super::service::MAX_GROUP_BYTES {
        return Err("answer and documents are too large to read".into());
    }
    let level = level(&c, node)?;
    let conn = s3_login(s, &c, &level)?;
    let url = fetch_url(s, &c, &level, conn.as_ref())?;
    let mut out = json!({"connection": c.id, "node": node});
    if b(p, "probe").unwrap_or(false) {
        out["fetch"] = json!(url);
        return Ok(out);
    }
    let found = match &level {
        Level::Database => database_schemas(s, &c, &mut out)?,
        Level::DatabaseSchema { schema } => database_schema(s, &c, schema, &mut out)?,
        Level::DatabaseDataset { dataset } => database_dataset(s, &c, dataset, &mut out)?,
        Level::Service { url, kind } => service(s, p, &mut out, url, kind.as_deref())?,
        Level::S3Root if url.is_none() => {
            buckets(c.options.get("buckets").map(String::as_str).unwrap_or("").split(',').map(str::to_string))
        }
        _ => {
            let at = url.as_deref().unwrap_or("");
            let text: Cow<str> = match ps(p, "answer") {
                Some(a) => Cow::Borrowed(a),
                None => Cow::Owned(download(at)?),
            };
            let bytes = text.as_bytes();
            match &level {
                Level::Catalogue => catalogue(s, &c, &text, &mut out)?,
                Level::StacRoot { api } if stac::catalog::is_static(api) => catalogs(bytes, api)?,
                Level::StacRoot { .. } => collections(bytes)?,
                Level::StacCatalog { url } => catalogs(bytes, url)?,
                Level::StacItem { url } => item_file(bytes, url)?,
                Level::StacCollection { .. } => items(bytes, at)?,
                Level::S3Root => buckets(s3::parse_buckets(bytes)?.into_iter()),
                Level::S3Prefix { bucket, prefix } => {
                    let binding = conn.as_ref().and_then(|conn| super::saved_binding(s, &c, conn));
                    objects(bytes, bucket, prefix, binding.as_deref())?
                }
                Level::Database | Level::DatabaseSchema { .. } | Level::DatabaseDataset { .. } | Level::Service { .. } => Found::default(),
            }
        }
    };
    out["children"] = json!(found.children);
    if found.truncated {
        out["truncated"] = json!(true);
    }
    if let Some(m) = found.matched {
        out["matched"] = json!(m);
    }
    Ok(out)
}

/// A live database's rows, with a note when it is a geodatabase whose catalog couldn't be read
/// (it then browses as plain SQL Server).
fn database_rows(s: &mut Session, c: &Connection, out: &mut Value) -> Result<Vec<Value>, String> {
    let (rows, _, problem) = super::connection_use::tables_and_geodatabase(s, c, None)?;
    if let Some(why) = problem {
        out["notes"] = json!([format!("the geodatabase catalog couldn't be read: {why}")]);
    }
    Ok(rows)
}

/// A live database's schemas, each browsed separately so one large schema cannot hide later ones;
/// a geodatabase's feature datasets, classes and tables instead.
fn database_schemas(s: &mut Session, c: &Connection, out: &mut Value) -> Result<Found, String> {
    let rows = database_rows(s, c, out)?;
    Ok(if is_geodatabase(&rows) { geodatabase_tree(&rows, &c.address, None) } else { schemas(&rows) })
}

/// Whether a database's rows are an Esri geodatabase's: each says what item it is.
fn is_geodatabase(rows: &[Value]) -> bool {
    rows.iter().any(|r| r.get("item").is_some())
}

/// A geodatabase's classes and tables as `connection.browse` lists them: the root has the
/// `Feature datasets` folder, the classes outside any dataset, a `Tables` folder of the
/// registered tables and an `Other spatial tables` folder (the empty ones left out); the node
/// of a feature dataset (`Some(its name)`) lists its classes.
fn geodatabase_tree(rows: &[Value], address: &str, node: Option<&str>) -> Found {
    let text = |r: &Value, k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let item = |r: &Value| r.get("item").and_then(Value::as_str).unwrap_or("").to_string();
    let child = |r: &Value, name: String, detail: String| {
        let label = text(r, "label");
        json!({
            "key": label,
            "name": name,
            "what": "table",
            "detail": detail,
            "add": {"command": "layer.addDatabase", "params": {"database": address, "layer": label}},
            "options": true,
        })
    };
    let shown = |r: &Value| match text(r, "display") {
        d if d.is_empty() => text(r, "name"),
        d => d,
    };
    // Alphabetical by what is shown (the alias, else the name).
    let by_name = |r: &Value| (shown(r).to_lowercase(), text(r, "label"));
    fn sorted<'a>(mut rows: Vec<&'a Value>, key: &impl Fn(&Value) -> (String, String)) -> Vec<&'a Value> {
        rows.sort_by_cached_key(|r| key(r));
        rows
    }
    let class_child = |r: &Value| {
        let detail = match text(r, "versioning").as_str() {
            v @ ("traditional" | "branch") => format!("feature class · {v}"),
            _ => "feature class".to_string(),
        };
        child(r, shown(r), detail)
    };
    let dataset_of = |r: &Value| r.get("dataset").and_then(Value::as_str).filter(|d| !d.is_empty()).map(str::to_string);
    let classes = || rows.iter().filter(|r| item(r) == "featureClass");

    if let Some(dataset) = node {
        let inside = classes().filter(|r| dataset_of(r).as_deref() == Some(dataset)).collect();
        return Found { children: sorted(inside, &by_name).into_iter().map(class_child).collect(), ..Found::default() };
    }

    let mut children = Vec::new();
    let datasets: std::collections::BTreeSet<String> = classes().filter_map(dataset_of).collect();
    if !datasets.is_empty() {
        let listed: Vec<Value> = datasets
            .iter()
            .map(|name| {
                let mut node = String::with_capacity(8 + name.len().div_ceil(3) * 4);
                node.push_str("dataset:");
                URL_SAFE_NO_PAD.encode_string(name, &mut node);
                json!({"key": node, "node": node, "name": name, "what": "dataset"})
            })
            .collect();
        children.push(json!({"key": "datasets", "name": "Feature datasets", "what": "folder", "children": listed}));
    }
    children.extend(sorted(classes().filter(|r| dataset_of(r).is_none()).collect(), &by_name).into_iter().map(class_child));
    let tables: Vec<Value> = sorted(rows.iter().filter(|r| item(r) == "table").collect(), &by_name)
        .into_iter()
        .map(|r| child(r, shown(r), "table".into()))
        .collect();
    if !tables.is_empty() {
        children.push(json!({"key": "tables", "name": "Tables", "what": "folder", "children": tables}));
    }
    // Several geometry columns of one table are told apart by the label (`name:column`).
    let other: Vec<Value> = rows.iter().filter(|r| item(r) == "other").map(|r| child(r, text(r, "label"), text(r, "kind"))).collect();
    if !other.is_empty() {
        children.push(json!({"key": "other", "name": "Other spatial tables", "what": "folder", "children": other}));
    }
    Found { children, ..Found::default() }
}

/// A geodatabase feature dataset's classes.
fn database_dataset(s: &mut Session, c: &Connection, dataset: &str, out: &mut Value) -> Result<Found, String> {
    let rows = database_rows(s, c, out)?;
    if !is_geodatabase(&rows) {
        return Err(format!("that isn't a node of {}", c.name));
    }
    Ok(geodatabase_tree(&rows, &c.address, Some(dataset)))
}

/// The names of every schema represented by the database's geometry tables.
fn schemas(tables: &[Value]) -> Found {
    let mut names = std::collections::BTreeSet::new();
    for table in tables {
        if let Some(schema) = table.get("schema").and_then(Value::as_str) {
            names.insert(schema);
        }
    }
    Found {
        children: names
            .into_iter()
            .map(|schema| {
                let mut node = String::with_capacity(7 + schema.len().div_ceil(3) * 4);
                node.push_str("schema:");
                URL_SAFE_NO_PAD.encode_string(schema, &mut node);
                json!({"key": node, "node": node, "name": schema, "what": "schema"})
            })
            .collect(),
        ..Found::default()
    }
}

/// A live database schema's geometry tables.
fn database_schema(s: &mut Session, c: &Connection, schema: &str, out: &mut Value) -> Result<Found, String> {
    let tables = database_rows(s, c, out)?;
    Ok(schema_tables(&tables, schema, &c.address))
}

/// Every geometry table in the opened schema; lazy expansion must not hide tables after a cap.
fn schema_tables(tables: &[Value], schema: &str, address: &str) -> Found {
    let text = |t: &Value, k: &str| t.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let mut children = Vec::new();
    for t in tables.iter().filter(|t| t.get("schema").and_then(Value::as_str) == Some(schema)) {
        let label = text(t, "label");
        let name = text(t, "name");
        let shown = if label.contains(':') { format!("{name}:{}", text(t, "column")) } else { name };
        children.push(json!({
            "key": label,
            "name": shown,
            "what": "table",
            "detail": text(t, "kind"),
            "add": {"command": "layer.addDatabase", "params": {"database": address, "layer": label}},
            "options": true,
        }));
    }
    Found { children, ..Found::default() }
}

/// A service's layers (capabilities read here, or the app's `answer` and `documents`).
fn service(s: &mut Session, p: &Value, out: &mut Value, url: &str, kind: Option<&str>) -> Result<Found, String> {
    let mut q = json!({"url": url});
    if let Some(k) = kind {
        q["type"] = json!(k);
    }
    if let Some(a) = ps(p, "answer") {
        q["text"] = json!(a);
        if let Some(d) = p.get("documents") {
            q["documents"] = d.clone();
        }
    }
    let r = super::super::service::layers(s, &q)?;
    out["service"] = json!({"url": url, "type": kind.unwrap_or("auto")});
    if let Some(n) = r.get("notes").filter(|n| n.as_array().is_some_and(|a| !a.is_empty())) {
        out["notes"] = n.clone();
    }
    if let Some(next) = r.get("next").filter(|n| !n.is_null()) {
        out["next"] = next.clone();
    }
    // The dialog imports an OGC API's collections together: a features layer isn't added alone there.
    let grouped = r.get("kind").and_then(Value::as_str).is_some_and(|k| k.starts_with("OGC API"));
    let text = |l: &Value, k: &str| l.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let children = r
        .get("layers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_CHILDREN + 1)
        .map(|l| {
            let (name, title) = (text(l, "name"), text(l, "title"));
            let offer = l.get("offer").cloned().unwrap_or(Value::Null);
            let offer_type = offer.get("type").and_then(Value::as_str).unwrap_or("");
            if offer_type == "service" {
                let node = format!("service:arcgis:{}", text(&offer, "url"));
                if !node_ok(&node) {
                    return Value::Null;
                }
                let what = if name.ends_with('/') { "folder" } else { "service" };
                return json!({"key": node, "node": node, "name": title.trim_end_matches('/'), "what": what});
            }
            let what = match offer_type {
                "vectorTiles" => "vectorTiles",
                "features" => "features",
                "coverage" => "coverage",
                "style" => "style",
                _ => "map",
            };
            let source = text(l, "source_url");
            let mut params = json!({"url": url, "layer": name, "source_url": source});
            if let Some(k) = kind {
                params["type"] = json!(k);
            }
            let mut child = json!({
                "key": format!("{source}#{name}"),
                "name": if title.is_empty() { name.as_str() } else { title.as_str() },
                "what": what,
                "detail": name,
                "layer": l,
                "options": true,
            });
            if !(grouped && what == "features") {
                child["add"] = json!({"command": "service.add", "params": params});
            }
            child
        })
        .collect::<Vec<Value>>();
    // A folder or service whose address is too long to follow as a node is left out, and said so.
    let listed = children.len();
    let children: Vec<Value> = children.into_iter().filter(|c| !c.is_null()).collect();
    let left_out = children.len() < listed;
    let mut found = Found::new(children);
    found.truncated |= left_out;
    Ok(found)
}

/// A catalogue's records, each with the services and files it links to.
fn catalogue(s: &mut Session, c: &Connection, text: &str, out: &mut Value) -> Result<Found, String> {
    let r = super::super::service::catalog_search(s, &json!({"url": c.address, "text": text}))?;
    if let Some(n) = r.get("notes") {
        out["notes"] = n.clone();
    }
    let str_of = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let mut dropped = false;
    let children = r
        .get("records")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_CHILDREN + 1)
        .enumerate()
        .map(|(i, rec)| {
            let id = str_of(rec, "id");
            let title = str_of(rec, "title");
            let links: Vec<Value> = rec
                .get("links")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|l| {
                    let (url, kind, name) = (str_of(l, "url"), str_of(l, "kind"), str_of(l, "name"));
                    match kind.as_str() {
                        "wms" | "wmts" | "wfs" | "wcs" | "ogcapi" | "arcgis" => {
                            let node = format!("service:{kind}:{url}");
                            if !node_ok(&node) {
                                dropped = true;
                                return None;
                            }
                            Some(json!({"key": node, "node": node, "name": if name.is_empty() { url.as_str() } else { name.as_str() }, "what": "service", "detail": kind}))
                        }
                        "download" => {
                            let last = url.split(['?', '#']).next().unwrap_or("").trim_end_matches('/').rsplit('/').next().unwrap_or("");
                            let shown: String = if name.is_empty() { last } else { name.as_str() }.chars().take(200).collect();
                            let named: String = title.chars().take(200).collect();
                            Some(json!({
                                "key": url,
                                "name": shown,
                                "what": "download",
                                "add": {"command": "layer.addUrl", "params": {"url": url, "name": named}},
                            }))
                        }
                        _ => None,
                    }
                })
                .collect();
            let summary: String = str_of(rec, "abstract").chars().take(200).collect();
            json!({"key": format!("record:{}", if id.is_empty() { i.to_string() } else { id }), "name": title, "what": "record", "detail": summary, "children": links})
        })
        .collect();
    let mut found = Found::new(children);
    found.matched = r.get("matched").and_then(Value::as_u64);
    found.truncated |= dropped || r.get("next").is_some_and(|n| !n.is_null());
    Ok(found)
}

/// A STAC API's collections.
fn collections(bytes: &[u8]) -> Result<Found, String> {
    let list = stac::parse_collections(bytes)?;
    Ok(Found::new(
        list.into_iter()
            .take(MAX_CHILDREN + 1)
            .map(|c| {
                let node = format!("collection:{}", c.id);
                json!({"key": node, "node": node, "name": if c.title.is_empty() { c.id.as_str() } else { c.title.as_str() }, "what": "collection", "detail": c.id, "options": true})
            })
            .collect(),
    ))
}

/// What a static catalog's document lists below it: its catalogs and collections (`child` links)
/// and its items (`item` links), each a node read when it is opened (an item's file holds its
/// assets).
fn catalogs(bytes: &[u8], from: &str) -> Result<Found, String> {
    let mut dropped = false;
    let mut children = Vec::new();
    for (prefix, what, list) in [
        ("catalog", "catalog", stac::catalog::parse_children(bytes, from)?),
        ("item", "item", stac::catalog::parse_item_links(bytes, from)?),
    ] {
        for c in list {
            let node = format!("{prefix}:{}", c.url);
            if node_ok(&node) {
                children.push(json!({"key": node, "node": node, "name": c.title, "what": what}));
            } else {
                dropped = true;
            }
        }
    }
    let mut found = Found::new(children);
    found.truncated |= dropped;
    Ok(found)
}

/// The assets of an item that can be added, each with the `stac.addAsset` that adds it.
fn assets_of(item: &stac::Item) -> Vec<Value> {
    item.assets
        .iter()
        .take(MAX_CHILDREN)
        .filter_map(|a| {
            let (key, href) = item.asset_href(Some(&a.key)).ok()?;
            let mut params = json!({"asset": key, "name": format!("{} {key}", item.id)});
            match &item.url {
                Some(u) => params["url"] = json!(u),
                None => params["item"] = stac::item_json_with(item, false),
            }
            Some(json!({
                "key": format!("{}#{key}", item.url.as_deref().unwrap_or(&item.id)),
                "name": if a.title.is_empty() { key.as_str() } else { a.title.as_str() },
                "what": "asset",
                "detail": a.media_type,
                "href": href,
                "add": {"command": "stac.addAsset", "params": params},
            }))
        })
        .collect()
}

/// A static catalog's item (its own file): its assets.
fn item_file(bytes: &[u8], from: &str) -> Result<Found, String> {
    Ok(Found::new(assets_of(&stac::parse_item(bytes, from)?)))
}

/// The items of a collection's search, each with the assets that can be added.
fn items(bytes: &[u8], from: &str) -> Result<Found, String> {
    let (list, next) = stac::parse_items(bytes, from)?;
    let matched = stac::parse_matched(bytes);
    let truncated = next.is_some() || matched.is_some_and(|m| m > list.len() as u64);
    let children = list
        .iter()
        .take(MAX_CHILDREN)
        .map(|item| json!({"key": format!("item:{}", item.url.as_deref().unwrap_or(&item.id)), "name": item.id, "what": "item", "detail": item.datetime, "children": assets_of(item)}))
        .collect();
    Ok(Found { children, truncated, matched })
}

/// Buckets to browse, each a node.
fn buckets(names: impl Iterator<Item = String>) -> Found {
    Found::new(
        names
            .map(|n| n.trim().to_string())
            .filter(|n| s3::valid_bucket(n))
            .take(MAX_CHILDREN + 1)
            .map(|n| {
                let node = format!("s3:{n}/");
                json!({"key": node, "node": node, "name": n, "what": "bucket"})
            })
            .collect(),
    )
}

/// One level of a bucket: its folders (each a node) and its files. Through a saved connection's
/// login (`binding`), a file's add command carries an address that names it, so the layer is read
/// with that connection's keys and no other's; its key and name stay the plain object's.
fn objects(bytes: &[u8], bucket: &str, prefix: &str, binding: Option<&str>) -> Result<Found, String> {
    let list = s3::parse_list(bytes)?;
    let last = |k: &str| k.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_string();
    let folders = list.prefixes.iter().map(|p| {
        let node = format!("s3:{bucket}/{p}");
        json!({"key": node, "node": node, "name": format!("{}/", last(p)), "what": "prefix"})
    });
    let mut dropped = false;
    let files = list.objects.iter().filter(|o| o.key != prefix && !o.key.ends_with('/')).filter_map(|o| {
        let url = format!("s3://{bucket}/{}", o.key);
        let add = match binding {
            Some(b) => match s3::bound_url(b, &url) {
                Ok(bound) => bound,
                Err(_) => {
                    dropped = true;
                    return None;
                }
            },
            None => url.clone(),
        };
        Some(json!({
            "key": url,
            "name": last(&o.key),
            "what": "object",
            "detail": o.size.to_string(),
            "add": {"command": "layer.addUrl", "params": {"url": add}},
        }))
    });
    let mut found = Found::new(folders.chain(files).take(MAX_CHILDREN + 1).collect());
    found.truncated |= dropped || list.next.is_some();
    Ok(found)
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use serde_json::{Value, json};

    #[test]
    fn failed_saved_login_addresses_mark_the_object_listing_incomplete() {
        let xml = b"<ListBucketResult><Contents><Key>object.fgb</Key><Size>42</Size></Contents></ListBucketResult>";
        let result = super::objects(xml, "data", "", Some("not-a-live-binding")).unwrap();
        assert!(result.truncated);
        assert!(result.children.is_empty());
    }

    fn table(schema: &str, name: &str) -> Value {
        json!({"schema": schema, "name": name, "label": format!("{schema}.{name}"), "kind": "MultiPolygon"})
    }

    #[test]
    fn every_schema_and_its_tables_are_reachable_past_the_old_cap() {
        let mut tables: Vec<Value> = (1..=832).map(|i| table("public", &format!("t_{i:04}"))).collect();
        tables.extend(["addr", "edges", "zip"].map(|name| table("tiger", name)));

        let root = super::schemas(&tables);
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.children[0]["name"], "public");
        assert_eq!(root.children[1]["name"], "tiger");
        for child in &root.children {
            let node = child["node"].as_str().unwrap();
            let parsed =
                super::level(&crate::connections::Connection { kind: "postgres".into(), name: "gis".into(), ..Default::default() }, node)
                    .unwrap();
            assert!(matches!(parsed, super::Level::DatabaseSchema { .. }));
            assert!(
                super::level(&crate::connections::Connection { kind: "service".into(), name: "gis".into(), ..Default::default() }, node)
                    .is_err()
            );
        }

        let public = super::schema_tables(&tables, "public", "postgres:dbname=gis");
        assert_eq!(public.children.len(), 832);
        assert!(!public.truncated);
        assert_eq!(public.matched, None);
        assert_eq!(public.children[0]["name"], "t_0001");
        assert_eq!(public.children.last().unwrap()["add"]["params"]["layer"], "public.t_0832");
    }

    #[test]
    fn schema_nodes_round_trip_unusual_identifiers() {
        for name in ["line\nbreak", "tab\tname", "quote\"\\:name", "réseau\u{85}", "日本語"] {
            let tables = [table(name, "roads"), table("other", "parks")];
            let root = super::schemas(&tables);
            let child = root.children.iter().find(|child| child["name"] == name).unwrap();
            let node = child["node"].as_str().unwrap();
            assert!(super::node_ok(node), "generated schema node must pass command validation: {node:?}");
            for kind in ["postgres", "mysql", "sqlserver"] {
                let connection = crate::connections::Connection { kind: kind.into(), ..Default::default() };
                let super::Level::DatabaseSchema { schema } = super::level(&connection, node).unwrap() else {
                    panic!("schema node must select a database schema");
                };
                assert_eq!(schema, name);
                let found = super::schema_tables(&tables, &schema, "database");
                assert_eq!(found.children.len(), 1);
                assert_eq!(found.children[0]["add"]["params"]["layer"], format!("{name}.roads"));
            }
        }
    }

    #[test]
    fn invalid_encoded_schema_nodes_are_rejected() {
        let connection = crate::connections::Connection { kind: "postgres".into(), ..Default::default() };
        for node in ["schema:", "schema:!", "schema:_w", "schema:YQ=="] {
            assert!(super::level(&connection, node).is_err(), "{node}");
        }
    }

    #[test]
    fn opening_a_short_schema_lists_every_table_without_a_truncation_note() {
        let tables = vec![table("tiger", "addr"), table("tiger", "edges"), table("public", "roads")];
        let tiger = super::schema_tables(&tables, "tiger", "postgres:dbname=gis");
        assert_eq!(tiger.children.len(), 2);
        assert!(!tiger.truncated);
        assert_eq!(tiger.matched, None);
    }

    /// The rows `connection_use::tables` gives for a geodatabase (a class in a dataset, a class
    /// at the root, a registered table, a spatial table the catalog doesn't register).
    fn geodatabase_rows() -> Vec<Value> {
        vec![
            json!({"label": "sde.Mains", "schema": "sde", "name": "Mains", "column": "Shape", "kind": "MultiLineString", "item": "featureClass",
                   "dataset": "Water", "alias": "Water mains", "versioning": "traditional", "display": "Water mains"}),
            json!({"label": "sde.Valves", "schema": "sde", "name": "Valves", "column": "Shape", "kind": "Point", "item": "featureClass",
                   "dataset": null, "alias": null, "versioning": "none", "display": "Valves"}),
            json!({"label": "sde.Readings", "schema": "sde", "name": "Readings", "item": "table", "dataset": null, "alias": null, "display": "Readings"}),
            json!({"label": "dbo.Parcels", "schema": "dbo", "name": "Parcels", "column": "geom", "kind": "MultiPolygon", "item": "other"}),
        ]
    }

    fn names(found: &super::Found) -> Vec<&str> {
        found.children.iter().map(|c| c["name"].as_str().unwrap()).collect()
    }

    #[test]
    fn a_geodatabase_browses_by_dataset_class_and_table() {
        let rows = geodatabase_rows();
        let root = super::geodatabase_tree(&rows, "mssql://gis-db/utility", None);
        assert_eq!(names(&root), ["Feature datasets", "Valves", "Tables", "Other spatial tables"]);
        assert_eq!(root.children[0]["what"], "folder");
        let datasets = root.children[0]["children"].as_array().unwrap();
        assert_eq!(datasets.len(), 1);
        assert_eq!(datasets[0]["name"], "Water");
        assert_eq!(datasets[0]["what"], "dataset");
        let node = datasets[0]["node"].as_str().unwrap();
        assert_eq!(node, format!("dataset:{}", super::URL_SAFE_NO_PAD.encode("Water")));
        let valves = &root.children[1];
        assert_eq!(valves["what"], "table");
        assert_eq!(valves["detail"], "feature class");
        assert_eq!(valves["add"]["command"], "layer.addDatabase");
        assert_eq!(valves["add"]["params"]["layer"], "sde.Valves");
        assert_eq!(valves["add"]["params"]["database"], "mssql://gis-db/utility");
        let tables = root.children[2]["children"].as_array().unwrap();
        assert_eq!(tables[0]["name"], "Readings");
        assert_eq!(tables[0]["detail"], "table");
        assert_eq!(tables[0]["add"]["params"]["layer"], "sde.Readings");
        let other = root.children[3]["children"].as_array().unwrap();
        assert_eq!(other[0]["add"]["params"]["layer"], "dbo.Parcels");

        // The dataset's node lists its classes, by their aliases, and says how they are versioned.
        let connection = crate::connections::Connection { kind: "sqlserver".into(), ..Default::default() };
        let super::Level::DatabaseDataset { dataset } = super::level(&connection, node).unwrap() else {
            panic!("a dataset node selects a dataset");
        };
        assert_eq!(dataset, "Water");
        let water = super::geodatabase_tree(&rows, "mssql://gis-db/utility", Some(&dataset));
        assert_eq!(names(&water), ["Water mains"]);
        assert!(water.children[0]["detail"].as_str().unwrap().contains("traditional"));
        assert_eq!(water.children[0]["add"]["params"]["layer"], "sde.Mains");
        assert!(super::geodatabase_tree(&rows, "mssql://gis-db/utility", Some("Elsewhere")).children.is_empty());
    }

    #[test]
    fn a_geodatabase_leaves_out_empty_folders() {
        let rows: Vec<Value> = geodatabase_rows().into_iter().filter(|r| r["item"] == "featureClass" && r["dataset"].is_null()).collect();
        assert_eq!(names(&super::geodatabase_tree(&rows, "mssql://gis-db/utility", None)), ["Valves"]);
    }

    #[test]
    fn a_geodatabase_lists_layers_without_its_registered_tables() {
        let mut rows = geodatabase_rows();
        rows.push(
            json!({"label": "sde.Orphans", "schema": "sde", "name": "Orphans", "column": null, "kind": null, "item": "featureClass"}),
        );
        let result = super::super::connection_use::listing_result(&rows, Some(json!({"owner": "sde", "release": "11.3.0"})), None);
        assert_eq!(result["layers"], json!(["sde.Mains", "sde.Valves", "dbo.Parcels"]));
        assert_eq!(result["count"], 3);
        assert_eq!(result["geodatabase"]["owner"], "sde");
        assert!(result.get("geodatabaseProblem").is_none());
        let plain = super::super::connection_use::listing_result(&[table("public", "roads")], None, Some("no permission".into()));
        assert_eq!(plain["layers"], json!(["public.roads"]));
        assert_eq!(plain["geodatabaseProblem"], "no permission");
    }

    #[test]
    fn a_class_without_a_spatial_table_is_listed_but_is_not_a_layer() {
        use rustgis_data::source::geodatabase::{Item, ItemKind, Versioning};
        use rustgis_data::source::sqlserver::{GdbListing, Table};
        let item = |name: &str, kind| Item {
            name: format!("utility.sde.{name}"),
            schema: "sde".into(),
            table: name.to_uppercase(),
            kind,
            dataset: None,
            alias: None,
            shape_column: Some("Shape".into()),
            field_aliases: Vec::new(),
            versioning: Versioning::Unversioned,
            definition_ok: true,
            family: None,
            archived: false,
            rowid_column: None,
            srid: None,
        };
        let valves = Table {
            schema: "sde".into(),
            name: "VALVES".into(),
            column: "Shape".into(),
            kind: "geometry".into(),
            current: false,
            several: false,
            alias: None,
            dataset: None,
        };
        let listing = GdbListing {
            datasets: Vec::new(),
            classes: vec![(item("Valves", ItemKind::FeatureClass), Some(valves)), (item("Orphans", ItemKind::FeatureClass), None)],
            tables: vec![item("Readings", ItemKind::Table)],
            other: Vec::new(),
        };
        let rows = super::super::connection_use::geodatabase_rows(listing);
        let orphans = rows.iter().find(|r| r["name"] == "ORPHANS").unwrap();
        assert!(orphans["column"].is_null() && orphans["kind"].is_null());
        assert_eq!(orphans["label"], "sde.ORPHANS");
        let result = super::super::connection_use::listing_result(&rows, None, None);
        assert_eq!(result["layers"], json!(["sde.VALVES"]));
        assert_eq!(result["count"], 1);
        // Even a hand-built row that names a shape column isn't a layer without a geometry kind.
        let named = [json!({"label": "sde.Orphans", "column": "Shape", "kind": null, "item": "featureClass"})];
        assert!(super::super::connection_use::listing_result(&named, None, None)["layers"].as_array().unwrap().is_empty());
    }

    #[test]
    fn a_plain_database_still_browses_schemas() {
        let tables = vec![table("tiger", "addr"), table("public", "roads")];
        assert!(!super::is_geodatabase(&tables));
        assert!(super::is_geodatabase(&geodatabase_rows()));
        assert_eq!(names(&super::schemas(&tables)), ["public", "tiger"]);
    }
}
