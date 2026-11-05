//! Esri `.sde` connection files: what ArcGIS Pro saves for an enterprise geodatabase. The file is
//! an undocumented binary container that carries `KEY=VALUE;` connection properties as text
//! (ASCII and/or UTF-16LE), so the few we need are scanned for by name rather than parsed. Only
//! the server, database, user and version come out, enough to prefill a SQL Server connection;
//! the encrypted password is never read, so it can't be shown or logged. Pure text, so the
//! browser build has it too. The file is hostile input: its size is bounded by [`MAX_BYTES`].

/// Largest `.sde` file read.
pub const MAX_BYTES: usize = 1 << 20;

/// Longest value kept; a longer one is junk, not a server or a user.
const MAX_VALUE: usize = 1024;

/// What a `.sde` file says about where to connect and as whom (never the password).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SdeConnection {
    /// `sqlserver`, `postgresql`, `oracle` or another, in lower case: `DBCLIENT`, else the
    /// `sde:<platform>:` prefix of `INSTANCE`. Empty when the file doesn't say.
    pub platform: String,
    /// `SERVER`, else `DB_CONNECTION_PROPERTIES`, else `INSTANCE` after its `sde:<platform>:` prefix.
    pub server: String,
    /// `DATABASE`; empty when the file doesn't name one.
    pub database: String,
    /// `USER`; `None` with operating-system authentication.
    pub user: Option<String>,
    /// `AUTHENTICATION_MODE=OSA`: sign in as the Windows user, with no user name or password.
    pub os_auth: bool,
    /// `VERSION`, the geodatabase version the connection opens.
    pub version: Option<String>,
}

/// The connection a `.sde` file describes. An error is a sentence for the person, not a log line.
pub fn parse(bytes: &[u8]) -> Result<SdeConnection, String> {
    if bytes.len() > MAX_BYTES {
        return Err(format!("that file is too large to be a connection file (over {} MB)", MAX_BYTES >> 20));
    }
    let texts = Texts::new(bytes);
    let instance = texts.value("INSTANCE");
    let (instance_platform, instance_server) = instance.as_deref().map_or((None, None), split_instance);
    let platform = texts.value("DBCLIENT").or(instance_platform).map(|p| p.to_ascii_lowercase()).unwrap_or_default();
    let server = ["SERVER", "DB_CONNECTION_PROPERTIES"].iter().find_map(|k| texts.value(k)).or(instance_server);
    let Some(server) = server else {
        return Err("that connection file doesn't name a server; enter the server by hand".into());
    };
    let os_auth = texts.value("AUTHENTICATION_MODE").is_some_and(|m| m.eq_ignore_ascii_case("OSA"));
    Ok(SdeConnection {
        platform,
        server,
        database: texts.value("DATABASE").unwrap_or_default(),
        user: if os_auth { None } else { texts.value("USER") },
        os_auth,
        version: texts.value("VERSION"),
    })
}

/// The SQL Server address for a connection: `mssql://<user>@<host>[:port][/<database>][?instance=<i>]`.
/// `host\instance` gives `?instance=`, `host,port` gives `:port`. Any other platform is refused,
/// saying which it is.
pub fn mssql_address(c: &SdeConnection) -> Result<String, String> {
    if c.platform != "sqlserver" {
        return Err(format!("this is {}; only SQL Server geodatabases can be opened for now", platform_phrase(&c.platform)));
    }
    let (rest, port) = match c.server.trim().rsplit_once(',') {
        Some((rest, port)) => match port.trim().parse::<u16>() {
            Ok(p) if p > 0 => (rest, Some(p)),
            _ => return Err(format!("the server's port `{}` isn't a port number", port.trim())),
        },
        None => (c.server.trim(), None),
    };
    let (host, instance) = rest.split_once('\\').map_or((rest, None), |(h, i)| (h, Some(i)));
    let host = host.trim();
    if host.is_empty() || !host.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_')) {
        return Err("the server's name isn't one a connection address can carry; enter the server by hand".into());
    }
    let mut address = String::from("mssql://");
    if let Some(user) = c.user.as_deref().filter(|u| !u.is_empty() && !c.os_auth) {
        address.push_str(&percent_encode(user));
        address.push('@');
    }
    address.push_str(host);
    if let Some(port) = port {
        address.push_str(&format!(":{port}"));
    }
    if !c.database.is_empty() {
        address.push('/');
        address.push_str(&percent_encode(&c.database));
    }
    if let Some(instance) = instance.map(str::trim).filter(|i| !i.is_empty()) {
        address.push_str("?instance=");
        address.push_str(&percent_encode(instance));
    }
    Ok(address)
}

/// A platform for people, with its article: "a PostgreSQL geodatabase".
fn platform_phrase(platform: &str) -> String {
    match platform {
        "postgresql" => "a PostgreSQL geodatabase".into(),
        "oracle" => "an Oracle geodatabase".into(),
        "" => "a geodatabase of an unknown kind".into(),
        other => format!("a geodatabase on `{other}`"),
    }
}

/// `sde:<platform>:<server>` as (platform, server); a value without the prefix is all server.
fn split_instance(instance: &str) -> (Option<String>, Option<String>) {
    let mut parts = instance.splitn(3, ':');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(sde), Some(platform), Some(server)) if sde.eq_ignore_ascii_case("sde") => {
            let some = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
            (some(platform), some(server))
        }
        _ => (None, Some(instance.to_string())),
    }
}

/// Text with everything but unreserved characters as `%XX` (so `DOMAIN\user` is `DOMAIN%5Cuser`).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The file's bytes read as text every way it may hold the properties: ASCII, and UTF-16LE at
/// both byte alignments. Anything that isn't text becomes NUL, which ends a value.
struct Texts(Vec<Vec<char>>);

impl Texts {
    fn new(bytes: &[u8]) -> Self {
        let ascii = bytes.iter().map(|&b| if b.is_ascii() { b as char } else { '\0' }).collect();
        let utf16 = |skip: usize| {
            let units = bytes.get(skip..).unwrap_or_default().as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c));
            char::decode_utf16(units).map(|r| r.unwrap_or('\0')).collect()
        };
        Self(vec![ascii, utf16(0), utf16(1)])
    }

    /// The first non-empty value of `key` (in any case) after `KEY=` and before the next `;`,
    /// control character or secret's name. The key starts a pair: at the start of the text or
    /// after a character that can't be part of another key.
    fn value(&self, key: &str) -> Option<String> {
        self.0.iter().find_map(|text| text_value(text, key))
    }
}

fn text_value(text: &[char], key: &str) -> Option<String> {
    let key: Vec<char> = key.chars().collect();
    (0..text.len()).find_map(|at| {
        let starts_pair = at == 0 || text.get(at - 1).is_some_and(|c| !(c.is_ascii_alphanumeric() || *c == '_'));
        let named = text.get(at..at + key.len()).is_some_and(|w| w.iter().zip(&key).all(|(a, b)| a.eq_ignore_ascii_case(b)));
        if !starts_pair || !named || text.get(at + key.len()) != Some(&'=') {
            return None;
        }
        let rest = text.get(at + key.len() + 1..)?;
        // Look no further than a value can be long, so a pair start costs O(MAX_VALUE), not O(file).
        let end = match rest.iter().take(MAX_VALUE + 1).position(|&c| c == ';' || c.is_control()) {
            Some(end) => end,
            None if rest.len() <= MAX_VALUE => rest.len(),
            None => return None,
        };
        let mut value: String = rest.get(..end)?.iter().collect();
        let upper = value.to_ascii_uppercase();
        if let Some(secret) = ["ENCRYPTED_PASSWORD", "PASSWORD"].iter().filter_map(|k| upper.find(k)).min() {
            value.truncate(secret);
        }
        let value = value.trim();
        (!value.is_empty() && value.chars().count() <= MAX_VALUE).then(|| value.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASCII: &[u8] = b"\x00\x01SERVER=gis-db;INSTANCE=sde:sqlserver:gis-db;DBCLIENT=sqlserver;DB_CONNECTION_PROPERTIES=gis-db;DATABASE=utility;USER=editor;ENCRYPTED_PASSWORD=00ff;VERSION=sde.DEFAULT;AUTHENTICATION_MODE=DBMS;\x00";

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    fn connection(server: &str, user: Option<&str>, os_auth: bool) -> SdeConnection {
        SdeConnection {
            platform: "sqlserver".into(),
            server: server.into(),
            database: "utility".into(),
            user: user.map(String::from),
            os_auth,
            version: None,
        }
    }

    fn assert_ascii_result(c: &SdeConnection) {
        assert_eq!(c.platform, "sqlserver");
        assert_eq!(c.server, "gis-db");
        assert_eq!(c.database, "utility");
        assert_eq!(c.user.as_deref(), Some("editor"));
        assert!(!c.os_auth);
        assert_eq!(c.version.as_deref(), Some("sde.DEFAULT"));
    }

    #[test]
    fn ascii_pairs_parse() {
        assert_ascii_result(&parse(ASCII).unwrap());
    }

    #[test]
    fn utf16_pairs_parse() {
        let text = String::from_utf8_lossy(ASCII.get(2..ASCII.len() - 1).unwrap()).into_owned();
        for pad in [0, 1] {
            let mut bytes = vec![0xFF; 7 + pad];
            bytes.extend(utf16(&text));
            bytes.extend([0x13, 0x37, 0x00, 0xFF]);
            assert_ascii_result(&parse(&bytes).unwrap());
        }
    }

    #[test]
    fn os_auth_has_no_user() {
        let c = parse(b"SERVER=gis-db;DATABASE=utility;USER=ignored;AUTHENTICATION_MODE=OSA;").unwrap();
        assert!(c.os_auth);
        assert_eq!(c.user, None);
        let c = parse(b"SERVER=gis-db;DATABASE=utility;AUTHENTICATION_MODE=osa;").unwrap();
        assert!(c.os_auth && c.user.is_none());
    }

    #[test]
    fn instance_prefix_gives_platform_and_server() {
        let c = parse(b"INSTANCE=sde:postgresql:pg-host,5432;DATABASE=d").unwrap();
        assert_eq!(c.platform, "postgresql");
        assert_eq!(c.server, "pg-host,5432");
        assert_eq!(c.database, "d");
    }

    #[test]
    fn server_falls_back_to_connection_properties() {
        let c = parse(b"DB_CONNECTION_PROPERTIES=props-host;INSTANCE=sde:sqlserver:inst-host;DATABASE=d").unwrap();
        assert_eq!(c.server, "props-host");
        assert_eq!(c.platform, "sqlserver");
    }

    #[test]
    fn keys_match_only_at_a_boundary() {
        let c = parse(b"OS_USER=other;SERVER=gis-db;DATABASE=d;USER=editor;").unwrap();
        assert_eq!(c.user.as_deref(), Some("editor"));
        let c = parse(b"SERVER=gis-db;DATABASE=d;Server_Extra=x;user=low;").unwrap();
        assert_eq!(c.user.as_deref(), Some("low"));
    }

    #[test]
    fn no_server_is_refused() {
        let err = parse(b"DATABASE=d;USER=u").unwrap_err();
        assert!(err.contains("enter the server"), "{err}");
        assert!(parse(b"").is_err());
    }

    #[test]
    fn too_large_is_refused() {
        assert!(parse(&vec![b'x'; MAX_BYTES + 1]).is_err());
        assert!(parse(&vec![b'x'; MAX_BYTES]).is_err()); // right size, no server
    }

    #[test]
    fn secrets_never_come_out() {
        let shown = format!("{:?}", parse(ASCII).unwrap());
        assert!(!shown.contains("00ff") && !shown.to_uppercase().contains("PASSWORD"), "{shown}");
        // a value that runs into a secret (a missing `;`) stops before it
        let c = parse(b"SERVER=gis-db;DATABASE=d;USER=editor ENCRYPTED_PASSWORD=00ff;").unwrap();
        let shown = format!("{c:?}");
        assert!(!shown.contains("00ff") && !shown.to_uppercase().contains("PASSWORD"), "{shown}");
        let c = parse(b"SERVER=gis-db;PASSWORD=hunter2;DATABASE=d;").unwrap();
        assert!(!format!("{c:?}").contains("hunter2"));
    }

    #[test]
    fn repeated_keys_without_a_terminator_are_cheap() {
        // Every `SERVER=` here starts a pair and none ends, in a file just under the size cap: the
        // scan from each must stop after a value's length, not run to the end of the file.
        let bytes = "SERVER=".repeat(MAX_BYTES / 7);
        assert!(bytes.len() <= MAX_BYTES);
        let started = std::time::Instant::now();
        let parsed = parse(bytes.as_bytes());
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
        // the pairs near the end have short tails, so a bounded junk server may come out; never a long one
        assert!(parsed.map_or(true, |c| c.server.chars().count() <= MAX_VALUE));
    }

    #[test]
    fn addresses_compose() {
        let a = |c: &SdeConnection| mssql_address(c).unwrap();
        assert_eq!(a(&connection(r"gis-db\GIS", Some("editor"), false)), "mssql://editor@gis-db/utility?instance=GIS");
        assert_eq!(a(&connection("gis-db,1434", Some("editor"), false)), "mssql://editor@gis-db:1434/utility");
        assert_eq!(a(&connection("gis-db", None, true)), "mssql://gis-db/utility");
        assert_eq!(a(&connection("gis-db", Some(r"CORP\ana"), false)), "mssql://CORP%5Cana@gis-db/utility");
        let mut c = connection("gis-db", Some("editor"), false);
        c.database = String::new();
        assert_eq!(a(&c), "mssql://editor@gis-db");
        c.platform = "postgresql".into();
        let err = mssql_address(&c).unwrap_err();
        assert!(err.contains("a PostgreSQL geodatabase"), "{err}");
        c.platform = "oracle".into();
        assert!(mssql_address(&c).unwrap_err().contains("an Oracle geodatabase"));
    }

    #[test]
    fn hostile_servers_are_refused() {
        for server in ["", r"\GIS", "h/x", "h?x", "h@x", "h,notaport", "h,0", "h,70000", "h x,1"] {
            assert!(mssql_address(&connection(server, Some("u"), false)).is_err(), "{server:?}");
        }
    }
}
