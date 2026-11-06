//! Matching the enterprise geodatabase an imported layer came from to saved SQL Server
//! connections. Pure text, so the browser build has it too: the server ArcGIS Pro wrote
//! (`host`, `host\instance`, `host,port`, `sde:sqlserver:host`) is compared with the servers
//! saved `mssql://` addresses name, never by connecting.

use rustgis_data::source::{msconn, pgconn};
use rustgis_map::source::GeodatabaseOrigin;

/// SQL Server's port when neither the address nor the server text names a port or an instance.
const DEFAULT_PORT: u16 = 1433;

/// Which SQL Server a server text or address names, in a form two spellings of it share.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerKey {
    /// Lower-case, without the brackets of an IPv6 literal.
    pub host: String,
    pub port: Option<u16>,
    /// Lower-case.
    pub instance: Option<String>,
}

/// How well a saved connection fits a geodatabase origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Match {
    /// The same server and the same database.
    Exact,
    /// The same database name on another host: the likely connection when a server was renamed
    /// or is reached by another name or address.
    Near,
    No,
}

fn finish(host: &str, port: Option<u16>, instance: Option<&str>) -> ServerKey {
    let host = host.trim().trim_matches(['[', ']']).to_ascii_lowercase();
    let instance = instance.map(|i| i.trim().to_ascii_lowercase()).filter(|i| !i.is_empty());
    ServerKey { host, port: if port.is_none() && instance.is_none() { Some(DEFAULT_PORT) } else { port }, instance }
}

/// Splits `host`, `host:port` and `[v6]:port` (a bare IPv6 literal is all host).
fn host_port(text: &str) -> Option<(&str, &str)> {
    if let Some(v6) = text.strip_prefix('[') {
        let (host, after) = v6.split_once(']')?;
        return Some((host, after.strip_prefix(':').unwrap_or("")));
    }
    if text.matches(':').count() > 1 {
        return Some((text, ""));
    }
    Some(text.rsplit_once(':').unwrap_or((text, "")))
}

/// The pieces of Pro's server text: the host as written, the port text if any, and the instance.
fn split_server(server: &str) -> (&str, Option<&str>, Option<&str>) {
    let mut text = server.trim();
    if text.get(..4).is_some_and(|p| p.eq_ignore_ascii_case("sde:"))
        && let Some((_, rest)) = text.get(4..).and_then(|r| r.split_once(':'))
    {
        text = rest.trim();
    }
    let (name, comma_port) = match text.rsplit_once(',') {
        Some((name, port)) => (name, Some(port)),
        None => (text, None),
    };
    let (name, instance) = match name.split_once('\\') {
        Some((name, instance)) => (name, Some(instance)),
        None => (name, None),
    };
    let (host, colon_port) = host_port(name.trim()).unwrap_or((name, ""));
    let port = comma_port.or(Some(colon_port).filter(|p| !p.is_empty()));
    (host, port, instance)
}

/// The server text ArcGIS Pro wrote for a geodatabase: `host`, `host\inst`, `host,port`,
/// `host:port`, or any of these after `sde:sqlserver:`. A port that isn't a number is ignored;
/// with neither a port nor an instance the default port, 1433, is the one named. Text that
/// names no host gives an empty host, which [`matches`] never matches.
pub fn server_key(server: &str) -> ServerKey {
    let (host, port, instance) = split_server(server);
    finish(host, port.and_then(|p| p.trim().parse::<u16>().ok()), instance)
}

/// The server key of an origin's server text, `None` when it names no host or a port that isn't
/// a number: a guess at the server is never evidence enough to offer a saved connection.
fn origin_server(server: &str) -> Option<ServerKey> {
    let (host, port, _) = split_server(server);
    let key = server_key(server);
    let port_ok = port.is_none_or(|p| p.trim().parse::<u16>().is_ok());
    (!host.trim().trim_matches(['[', ']']).is_empty() && port_ok).then_some(key)
}

/// The server and lower-case database a saved `mssql://` address names; `None` for another
/// scheme, an address that names no database, or one the connection itself would refuse to read.
/// Split as the desktop's connection reader does: the user and password are set aside, the host
/// and port come from the authority, and the `instance` query option is the instance.
pub fn address_key(address: &str) -> Option<(ServerKey, String)> {
    let (public, _) = msconn::split_password(address)?;
    let database = msconn::database_name(&public)?.to_lowercase();
    let (_, rest) = public.split_once("://")?;
    let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
    let hostport = match pgconn::userinfo_end(rest) {
        Some(at) => rest.get(at + 1..)?,
        None => rest,
    };
    let hostport = hostport.split('/').next().unwrap_or("");
    let (host, port) = host_port(hostport)?;
    let port = if port.is_empty() { None } else { Some(port.parse::<u16>().ok()?) };
    // An address that names no host reads the local server, as the connection reader does.
    let host = if host.trim().is_empty() { "localhost" } else { host };
    let mut instance = None;
    for kv in query.split('&').filter(|kv| !kv.is_empty()) {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        if k.to_ascii_lowercase().replace('_', "-") == "instance" {
            instance = Some(v);
        }
    }
    Some((finish(host, port, instance), database))
}

/// How an address fits a geodatabase origin. Only a SQL Server geodatabase (or one whose
/// platform isn't said) can match, and the database must be named on both sides. The server must
/// agree on host, port and instance for an `Exact` match; the same database on another host is
/// `Near`; the same host with another port or instance is another server, so `No`. The address's
/// user is ignored. An origin whose server names no host, or a port that isn't a number, is `No`.
pub fn matches(origin: &GeodatabaseOrigin, address: &str) -> Match {
    if !(origin.platform.is_empty() || origin.platform.eq_ignore_ascii_case("sqlserver")) || origin.database.trim().is_empty() {
        return Match::No;
    }
    let Some((saved, database)) = address_key(address) else { return Match::No };
    if !database.eq_ignore_ascii_case(origin.database.trim()) {
        return Match::No;
    }
    let Some(wanted) = origin_server(&origin.server) else { return Match::No };
    if wanted == saved {
        Match::Exact
    } else if wanted.host != saved.host {
        Match::Near
    } else {
        Match::No
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connections::{Connection, Connections};

    fn key(host: &str, port: Option<u16>, instance: Option<&str>) -> ServerKey {
        ServerKey { host: host.into(), port, instance: instance.map(Into::into) }
    }

    fn origin(platform: &str, server: &str, database: &str) -> GeodatabaseOrigin {
        GeodatabaseOrigin { platform: platform.into(), server: server.into(), database: database.into(), ..GeodatabaseOrigin::default() }
    }

    #[test]
    fn server_keys_normalise() {
        assert_eq!(server_key("GIS-DB\\Inst"), key("gis-db", None, Some("inst")));
        assert_eq!(server_key("gis-db,1434"), key("gis-db", Some(1434), None));
        assert_eq!(server_key("gis-db:1434"), key("gis-db", Some(1434), None));
        assert_eq!(server_key("gis-db"), key("gis-db", Some(1433), None));
        assert_eq!(server_key("sde:sqlserver:gis-db"), key("gis-db", Some(1433), None));
        assert_eq!(server_key("SDE:SqlServer:GIS-DB\\Inst"), key("gis-db", None, Some("inst")));
        assert_eq!(server_key(" gis-db,x "), key("gis-db", Some(1433), None));
        assert_eq!(server_key("sde:sqlserver:").host, "");
        assert_eq!(server_key("[::1]:1434"), key("::1", Some(1434), None));
    }

    #[test]
    fn addresses_match_pro_servers() {
        let pro = origin("sqlserver", "GIS-DB", "Utility");
        assert_eq!(matches(&pro, "mssql://sde@gis-db:1433/utility"), Match::Exact);
        assert_eq!(matches(&pro, "mssql://sde:pw@GIS-DB/Utility"), Match::Exact);
        assert_eq!(matches(&pro, "mssql://other@gis-db/utility?instance=x"), Match::No);
        assert_eq!(matches(&pro, "mssql://sde@gis-db:1434/utility"), Match::No);
        assert_eq!(matches(&pro, "mssql://sde@10.0.0.5/utility"), Match::Near);
        assert_eq!(matches(&pro, "mssql://sde@gis-db/other"), Match::No);
        assert_eq!(matches(&pro, "mssql://sde@gis-db"), Match::No);
        assert_eq!(matches(&pro, "postgres:host=gis-db dbname=utility"), Match::No);
        assert_eq!(matches(&origin("", "gis-db", "utility"), "mssql://sde@gis-db/utility"), Match::Exact);
        assert_eq!(matches(&origin("sqlserver", "gis-db", ""), "mssql://sde@gis-db/utility"), Match::No);
        let inst = origin("sqlserver", "gis-db\\inst", "utility");
        assert_eq!(matches(&inst, "mssql://sde@gis-db/utility?instance=INST"), Match::Exact);
        assert_eq!(matches(&inst, "mssql://sde@[::1]/utility?Instance=inst"), Match::Near);
        // A server that names no host, or a malformed port, is no evidence of any server.
        for server in ["", "  ", "sde:sqlserver:", ",1433", "gis-db,abc", "sde:sqlserver:gis-db:x"] {
            let blank = origin("sqlserver", server, "utility");
            for address in ["mssql://sde@localhost/utility", "mssql://sde@gis-db/utility", "mssql:///utility"] {
                assert_eq!(matches(&blank, address), Match::No, "{server:?} {address}");
            }
        }
        // A short name and its fully qualified name are not proof of the same server.
        let fqdn = origin("sqlserver", "gis-db.corp.example", "utility");
        assert_eq!(matches(&fqdn, "mssql://sde@gis-db/utility"), Match::Near);
        assert_eq!(matches(&pro, "mssql://sde@gis-db.corp.example/utility"), Match::Near);
        for platform in ["postgresql", "oracle"] {
            let other = origin(platform, "gis-db", "utility");
            for address in ["mssql://sde@gis-db/utility", "mssql://sde@10.0.0.5/utility"] {
                assert_eq!(matches(&other, address), Match::No, "{platform} {address}");
            }
        }
    }

    #[test]
    fn address_keys_split_like_the_connection_reader() {
        assert_eq!(address_key("mssql://sde:pw@GIS-DB:1434/Utility"), Some((key("gis-db", Some(1434), None), "utility".into())));
        assert_eq!(address_key("mssql://sde@gis-db/utility?instance=Inst"), Some((key("gis-db", None, Some("inst")), "utility".into())));
        assert_eq!(address_key("mssql://sde@[fe80::1]:1433/utility"), Some((key("fe80::1", Some(1433), None), "utility".into())));
        assert_eq!(address_key("mssql://gis-db/utility"), Some((key("gis-db", Some(1433), None), "utility".into())));
        assert_eq!(address_key("mssql://gis-db:abc/utility"), None);
        assert_eq!(address_key("postgres://sde@gis-db/utility"), None);
    }

    #[test]
    fn connections_are_matched_in_order() {
        let mut saved = Connections::default();
        for (id, address) in [
            ("c1", "mssql://sde@gis-db/utility"),
            ("c2", "mssql://sde@10.0.0.5/utility"),
            ("c3", "mssql://reader@GIS-DB:1433/Utility"),
            ("c4", "mssql://sde@gis-db/other"),
        ] {
            saved.connections.push(Connection {
                id: id.into(),
                name: id.into(),
                kind: "sqlserver".into(),
                address: address.into(),
                ..Connection::default()
            });
        }
        // The right address under another kind is not a SQL Server connection.
        saved.connections.push(Connection {
            id: "c5".into(),
            kind: "postgres".into(),
            address: "mssql://sde@gis-db/utility".into(),
            ..Connection::default()
        });
        let pro = origin("sqlserver", "gis-db", "utility");
        let ids = |v: Vec<&Connection>| v.into_iter().map(|c| c.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(saved.matching_geodatabase(&pro)), ["c1", "c3"]);
        assert_eq!(ids(saved.near_geodatabase(&pro)), ["c2"]);
        assert!(saved.matching_geodatabase(&origin("oracle", "gis-db", "utility")).is_empty());
    }
}
