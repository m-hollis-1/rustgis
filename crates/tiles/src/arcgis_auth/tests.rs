use super::*;

// The sessions are shared by the process's tests: each test uses its own hosts.

/// A plugin's token handed over replaces every one handed over before: the tests that hand one
/// over take turns.
fn adopting() -> std::sync::MutexGuard<'static, ()> {
    static ONE: Mutex<()> = Mutex::new(());
    ONE.lock().unwrap_or_else(|e| e.into_inner())
}

fn grant(token: &str, expires_at: i64) -> Grant {
    Grant { access_token: token.into(), refresh_token: Some("refresh".into()), expires_at, username: "ana".into() }
}

#[test]
fn portal_bases() {
    assert_eq!(portal_base("").unwrap(), ONLINE);
    assert_eq!(portal_base("https://arcgis.com/home").unwrap(), ONLINE);
    assert_eq!(portal_base("https://MyOrg.maps.arcgis.com/home/content.html").unwrap(), "https://myorg.maps.arcgis.com");
    assert_eq!(portal_base("https://gis.example.org/portal/sharing/rest?f=json").unwrap(), "https://gis.example.org/portal");
    assert_eq!(portal_base("https://gis.example.org:7443/arcgis/home/").unwrap(), "https://gis.example.org:7443/arcgis");
    assert_eq!(portal_base("https://gis.example.org:443/portal").unwrap(), "https://gis.example.org/portal");
    // Tokens travel to the portal: never over plain HTTP, never with user info.
    for bad in ["http://gis.example.org/portal", "https://user@gis.example.org/portal", "https://", "https://gis.example.org:x/", "ftp://a"]
    {
        assert!(portal_base(bad).is_err(), "{bad}");
    }
    assert!(is_online("https://myorg.maps.arcgis.com") && is_online(ONLINE) && !is_online("https://gis.example.org/portal"));
}

#[test]
fn endpoints_and_requests() {
    // An organization's users sign in on its page; its tokens come from www.arcgis.com.
    assert_eq!(endpoint("https://myorg.maps.arcgis.com", "authorize"), "https://myorg.maps.arcgis.com/sharing/rest/oauth2/authorize");
    assert_eq!(endpoint("https://myorg.maps.arcgis.com", "token"), "https://www.arcgis.com/sharing/rest/oauth2/token");
    assert_eq!(endpoint("https://gis.example.org/portal", "token"), "https://gis.example.org/portal/sharing/rest/oauth2/token");
    let url = authorize_url("https://gis.example.org/portal", "abc123", &redirect_uri(PORT), "chal", "st&te");
    assert!(url.starts_with("https://gis.example.org/portal/sharing/rest/oauth2/authorize?client_id=abc123&response_type=code"));
    assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A7290%2Farcgis%2Fcallback"));
    assert!(url.contains("code_challenge=chal&code_challenge_method=S256&state=st%26te"));
    let body = exchange_body("abc", "c/d", "http://127.0.0.1:7290/arcgis/callback", "v");
    assert!(body.contains("grant_type=authorization_code") && body.contains("code=c%2Fd") && body.contains("code_verifier=v"));
    assert!(refresh_body("abc", "r").contains("grant_type=refresh_token&client_id=abc&refresh_token=r"));
    assert!(revoke_body("abc", "r", true).ends_with("auth_token=r&token_type_hint=refresh_token"));
    // RFC 7636's example.
    assert_eq!(pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    assert!(valid_client_id("3IKyMtbdeceo0JCr") && !valid_client_id("") && !valid_client_id("a b") && !valid_client_id("a&b"));
}

#[test]
fn token_answers() {
    let g = parse_grant(br#"{"access_token":"tok.1","refresh_token":"ref","expires_in":1800,"username":"ana"}"#, 100).unwrap();
    assert_eq!(
        (g.access_token.as_str(), g.refresh_token.as_deref(), g.expires_at, g.username.as_str()),
        ("tok.1", Some("ref"), 1900, "ana")
    );
    // Refusals come as HTTP 200 with an error object.
    let e = parse_grant(br#"{"error":{"code":400,"error":"invalid_request","error_description":"Invalid PKCE code_verifier"}}"#, 0)
        .unwrap_err();
    assert!(e.contains("Invalid PKCE code_verifier"), "{e}");
    assert!(parse_grant(b"<html>", 0).is_err());
    assert!(parse_grant(br#"{"access_token":"bad token\r\nX: y"}"#, 0).is_err());
    // No lifetime: half an hour.
    assert_eq!(parse_grant(br#"{"access_token":"t"}"#, 0).unwrap().expires_at, 1800);
    // Its Debug form hides the tokens.
    assert!(!format!("{g:?}").contains("tok.1"));
}

#[test]
fn an_online_sign_in_covers_its_organization_only() {
    let portal = "https://scopeorg.maps.arcgis.com";
    sign_in(portal, "client", grant("online-token", now_secs() + 3600), scopes_for(portal, Some("Org1d"), &[]));
    let bearer = Some("Bearer online-token".to_string());
    assert_eq!(authorization("https://scopeorg.maps.arcgis.com/sharing/rest/content/items/x?f=json"), bearer);
    assert_eq!(authorization("https://services1.arcgis.com/Org1d/arcgis/rest/services/A/FeatureServer/0/query?f=json"), bearer);
    assert_eq!(authorization("https://tiles.arcgis.com/tiles/Org1d/arcgis/rest/services/B/MapServer/tile/1/0/0"), bearer);
    // Another organization's services, Esri's basemaps, look-alike hosts and plain HTTP don't get it.
    for url in [
        "https://services1.arcgis.com/0ther/arcgis/rest/services/A/FeatureServer",
        "https://services.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/1/0/0",
        "https://basemaps.arcgis.com/arcgis/rest/services/World_Basemap_v2/VectorTileServer",
        "https://services1.arcgis.com.evil.example/Org1d/arcgis/rest/services/A/FeatureServer",
        "https://evil.example/Org1d/arcgis/rest/services/A/FeatureServer",
        "http://services1.arcgis.com/Org1d/arcgis/rest/services/A/FeatureServer",
        "https://user@services1.arcgis.com/Org1d/arcgis/rest/services/A/FeatureServer",
        "https://services1.arcgis.com/x/y/Org1d/A/FeatureServer",
    ] {
        assert_eq!(authorization(url), None, "{url}");
    }
    assert!(signed_in(portal).is_some_and(|i| i.username == "ana" && i.renews && !i.token));
    let (revocations, gone) = sign_out(Some(portal));
    assert_eq!(gone, 1);
    assert!(revocations.len() == 1 && revocations[0].endpoint == "https://www.arcgis.com/sharing/rest/oauth2/revokeToken");
    assert_eq!(authorization("https://services1.arcgis.com/Org1d/arcgis/rest/services/A/FeatureServer"), None);
}

#[test]
fn an_enterprise_sign_in_covers_its_host_and_federated_servers() {
    let portal = "https://ent.example.org/portal";
    let servers = parse_servers(br#"{"servers":[{"url":"https://ent.example.org/server"},{"url":"https://maps.ent-other.example.org/arcgis"},{"url":"http://plain.example.org/server"}]}"#);
    assert_eq!(servers, ["https://ent.example.org/server", "https://maps.ent-other.example.org/arcgis"]);
    sign_in(portal, "client", grant("ent-token", now_secs() + 3600), scopes_for(portal, None, &servers));
    let bearer = Some("Bearer ent-token".to_string());
    assert_eq!(authorization("https://ent.example.org/server/rest/services/Roads/MapServer?f=json"), bearer);
    assert_eq!(authorization("https://maps.ent-other.example.org/ArcGIS/rest/services/A/FeatureServer"), bearer);
    assert_eq!(authorization("https://maps.ent-other.example.org/other/rest/services"), None);
    assert_eq!(authorization("https://ent.example.org.evil.example/server/rest/services"), None);
    // An expired token without renewal isn't sent.
    sign_in(portal, "client", grant("old", now_secs() - 1), scopes_for(portal, None, &[]));
    assert_eq!(authorization("https://ent.example.org/server/rest/services"), None);
    sign_out(Some(portal));
}

#[test]
fn a_token_in_a_service_address_reaches_its_layers() {
    // The reported case (#914): a service's address with ?token=… works in the browser, but its
    // layers, tiles and feature pages are fetched from addresses built without it.
    let url = "https://tokenserver.example.org/server/rest/services/Secure/MapServer?f=json&token=abc.DEF-123";
    let kept = adopt_url_token(url).unwrap();
    assert_eq!(kept, "https://tokenserver.example.org/server/rest/services/Secure/MapServer?f=json");
    let bearer = Some("Bearer abc.DEF-123".to_string());
    assert_eq!(authorization("https://tokenserver.example.org/server/rest/services/Secure/MapServer/tile/3/2/1"), bearer);
    assert_eq!(
        authorization("https://tokenserver.example.org/server/rest/services/Secure/MapServer/0/query?where=1%3D1&f=geojson"),
        bearer
    );
    assert_eq!(authorization("https://tokenserver.example.org/other/rest/services"), None);
    // A path leading elsewhere by its dot segments gets none.
    assert_eq!(authorization("https://tokenserver.example.org/server/../other/rest/services"), None);
    assert_eq!(authorization("https://tokenserver.example.org/server/%2E%2E/other/rest/services"), None);
    assert!(list().iter().any(|i| i.portal == "https://tokenserver.example.org/server" && i.token && i.expires_at.is_none()));
    // An address without a token is unchanged; a hostile one is refused.
    assert_eq!(
        adopt_url_token("https://a.example.org/server/rest/services?f=json").unwrap(),
        "https://a.example.org/server/rest/services?f=json"
    );
    assert!(adopt_url_token("https://b.example.org/server/rest/services?token=a%0D%0Ab").is_err());
    assert!(adopt_url_token("http://c.example.org/server/rest/services?token=abc").is_err());
    assert_eq!(
        adopt_url_token("https://d.example.org/server/rest/services/X/MapServer?token=").unwrap(),
        "https://d.example.org/server/rest/services/X/MapServer"
    );
    assert_eq!(
        server_root("https://services1.arcgis.com/Org/arcgis/rest/services/A/FeatureServer").as_deref(),
        Some("https://services1.arcgis.com/Org/arcgis/")
    );
    assert_eq!(server_root("https://e.example.org/rest/services").as_deref(), Some("https://e.example.org/"));
    assert_eq!(server_root("https://e.example.org/data.geojson"), None);
    sign_out(Some("https://tokenserver.example.org/server/"));
    assert_eq!(authorization("https://tokenserver.example.org/server/rest/services/Secure/MapServer/tile/3/2/1"), None);
}

#[test]
fn a_server_token_wins_over_a_portal_sign_in() {
    let portal = "https://both.example.org/portal";
    sign_in(portal, "client", grant("portal-token", now_secs() + 3600), scopes_for(portal, None, &[]));
    add_token("https://both.example.org/server/rest/services", "server-token").unwrap();
    assert_eq!(authorization("https://both.example.org/server/rest/services/A/MapServer").as_deref(), Some("Bearer server-token"));
    assert_eq!(authorization("https://both.example.org/portal/sharing/rest/portals/self").as_deref(), Some("Bearer portal-token"));
    // The browser sends it as the request's parameter instead (ArcGIS's query endpoints refuse
    // the header's CORS preflight).
    assert_eq!(
        with_token("https://both.example.org/server/rest/services/A/FeatureServer/0/query?where=1%3D1&f=geojson").as_deref(),
        Some("https://both.example.org/server/rest/services/A/FeatureServer/0/query?where=1%3D1&f=geojson&token=server-token")
    );
    assert_eq!(
        with_token("https://both.example.org/server/rest/services/A/MapServer/tile/1/0/0#x").as_deref(),
        Some("https://both.example.org/server/rest/services/A/MapServer/tile/1/0/0?token=server-token#x")
    );
    assert_eq!(with_token("https://elsewhere.example.org/server/rest/services"), None);
    // An address with a token of its own keeps it, alone.
    assert_eq!(with_token("https://both.example.org/server/rest/services/A/MapServer?token=users"), None);
    sign_out(Some(portal));
    sign_out(Some("https://both.example.org/server"));
    // A token for the portal's own address sits beside its sign-in, never replacing it.
    let portal = "https://same.example.org/portal";
    sign_in(portal, "client", grant("signin-token", now_secs() + 3600), scopes_for(portal, None, &[]));
    add_token("https://same.example.org/portal/sharing/rest/portals/self", "pasted-token").unwrap();
    let kinds: Vec<bool> = list().iter().filter(|i| i.portal == portal).map(|i| i.token).collect();
    assert!(kinds.len() == 2 && kinds.contains(&true) && kinds.contains(&false), "{kinds:?}");
    let (revocations, gone) = sign_out(Some(portal));
    assert_eq!((revocations.len(), gone), (1, 2));
}

#[test]
fn a_pasted_token_keeps_its_plus_signs() {
    adopt_url_token("https://plus.example.org/server/rest/services/A/MapServer?token=ab+cd%2Fef").unwrap();
    assert_eq!(authorization("https://plus.example.org/server/rest/services/A/MapServer/tile/0/0/0").as_deref(), Some("Bearer ab+cd/ef"));
    sign_out(Some("https://plus.example.org/server"));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_sign_in_refuses_bad_input_before_listening() {
    assert!(start_sign_in("http://gis.example.org/portal", "abc", 0).is_err());
    // An Enterprise portal registers its own app: no default.
    assert!(start_sign_in("https://gis.example.org/portal", "", 0).unwrap_err().contains("client id"));
}

#[test]
fn arcgis_online_has_a_default_client_id() {
    assert_eq!(client_id_for(ONLINE, "").as_deref(), Some(DEFAULT_CLIENT_ID));
    assert_eq!(client_id_for("https://myorg.maps.arcgis.com", " ").as_deref(), Some(DEFAULT_CLIENT_ID));
    assert_eq!(client_id_for("https://myorg.maps.arcgis.com", "mine").as_deref(), Some("mine"));
    assert_eq!(client_id_for("https://gis.example.org/portal", ""), None);
}

#[test]
fn a_browser_sign_in_reads_its_callback_page() {
    let redirect = "https://web.rustgis.app/oauth-callback.html";
    let w = web_sign_in("https://myorg.maps.arcgis.com", "", redirect, "verifier", "st8").unwrap();
    assert!(w.url.starts_with("https://myorg.maps.arcgis.com/sharing/rest/oauth2/authorize?client_id=nD2lKYqFqsaddfuD&"));
    assert!(w.url.contains("redirect_uri=https%3A%2F%2Fweb.rustgis.app%2Foauth-callback.html"));
    // Still on the portal's pages, or blank: not yet.
    assert!(w.exchange("about:blank").is_none() && w.exchange(redirect).is_none());
    assert!(w.exchange("https://evil.example/oauth-callback.html?code=c&state=st8").is_none());
    let (endpoint, body) = w.exchange(&format!("{redirect}?code=the%2Fcode&state=st8")).unwrap().unwrap();
    assert_eq!(endpoint, "https://www.arcgis.com/sharing/rest/oauth2/token");
    assert!(body.contains("code=the%2Fcode") && body.contains("code_verifier=verifier") && body.contains("client_id=nD2lKYqFqsaddfuD"));
    assert!(w.exchange(&format!("{redirect}?code=c&state=other")).unwrap().is_err());
    assert!(w.exchange(&format!("{redirect}?error=access_denied&state=st8")).unwrap().unwrap_err().contains("declined"));
    assert!(web_sign_in("https://gis.example.org/portal", "", redirect, "v", "s").is_err());
}

#[test]
fn renewals_land_or_end_the_session() {
    let portal = "https://renew.example.org/portal";
    let url = "https://renew.example.org/server/rest/services/A/MapServer";
    sign_in(portal, "client", grant("old", now_secs() + 10), scopes_for(portal, None, &[]));
    let r = renewal_due(url).unwrap();
    assert!(r.endpoint.ends_with("/portal/sharing/rest/oauth2/token") && r.body.contains("grant_type=refresh_token"));
    // Unreachable: kept, and not tried again at once.
    renewal_done(&r, Err("offline".into()));
    assert!(renewal_due(url).is_none() && authorization(url).is_some());
    // A proxy's error page: kept too.
    renewal_done(&r, Ok(b"<html>502 Bad Gateway</html>"));
    assert!(authorization(url).is_some());
    // A new access token.
    renewal_done(&r, Ok(br#"{"access_token":"new","expires_in":1800}"#));
    assert_eq!(authorization(url).as_deref(), Some("Bearer new"));
    // Refused: the session ends.
    sign_in(portal, "client", grant("old", now_secs() + 10), scopes_for(portal, None, &[]));
    let r = renewal_due(url).unwrap();
    renewal_done(&r, Ok(br#"{"error":{"code":400,"error":"invalid_grant"}}"#));
    assert!(authorization(url).is_none() && signed_in(portal).is_none());
}

#[test]
fn an_adopted_token_covers_the_organizations_hosted_services_only() {
    let _turn = adopting();
    let soon = now_secs() + 3600;
    assert_eq!(adopt(ONLINE, "plugin-token-1", soon, "abc123", "ana"), Ok(true));
    let bearer = Some("Bearer plugin-token-1".to_string());
    assert_eq!(authorization("https://services5.arcgis.com/abc123/arcgis/rest/services/S/FeatureServer/0"), bearer);
    assert_eq!(authorization("https://services5.arcgis.com/otherorg/arcgis/rest/services/S/FeatureServer/0"), None);
    assert_eq!(authorization("https://example.org/arcgis/rest/services/S/FeatureServer"), None);
    assert_eq!(authorization("https://example.org/abc123/arcgis/rest/services/S/FeatureServer"), None);
    // Shown as a sign-in with its username, which doesn't renew; never its token.
    let info = list().into_iter().find(|i| i.portal == ONLINE && i.username == "ana" && !i.renews && !i.token);
    assert!(info.is_some_and(|i| i.expires_at == Some(soon)));
    assert!(signed_in(ONLINE).is_some_and(|i| i.username == "ana"));
    // The same token again changes nothing; a new one replaces it.
    assert_eq!(adopt(ONLINE, "plugin-token-1", soon + 60, "abc123", "ana"), Ok(false));
    assert_eq!(adopt(ONLINE, "plugin-token-2", soon, "abc123", "ana"), Ok(true));
    assert_eq!(list().iter().filter(|i| i.portal == ONLINE).count(), 1);
    // The same token for another organization changes what it covers: changed.
    assert_eq!(adopt(ONLINE, "plugin-token-2", soon, "xyz789", "ana"), Ok(true));
    assert_eq!(
        authorization("https://services5.arcgis.com/xyz789/arcgis/rest/services/S/FeatureServer/0").as_deref(),
        Some("Bearer plugin-token-2")
    );
    assert_eq!(authorization("https://services5.arcgis.com/abc123/arcgis/rest/services/S/FeatureServer/0"), None);
    assert_eq!(adopt(ONLINE, "plugin-token-2", soon, "abc123", "ana"), Ok(true));
    assert_eq!(
        authorization("https://services5.arcgis.com/abc123/arcgis/rest/services/S/FeatureServer/0").as_deref(),
        Some("Bearer plugin-token-2")
    );
    // A sign-out forgets it, revoking nothing (the plugin's token is the plugin's).
    let (revocations, gone) = sign_out(Some(ONLINE));
    assert_eq!((revocations.len(), gone), (0, 1));
    assert_eq!(authorization("https://services5.arcgis.com/abc123/arcgis/rest/services/S/FeatureServer/0"), None);
}

#[test]
fn adopting_never_replaces_the_users_own_sign_in() {
    let _turn = adopting();
    let portal = "https://adoptown.maps.arcgis.com";
    let url = "https://services1.arcgis.com/own456/arcgis/rest/services/A/FeatureServer/0";
    sign_in(portal, "client", grant("users-token", now_secs() + 3600), scopes_for(portal, Some("own456"), &[]));
    assert_eq!(adopt(portal, "plugin-token", now_secs() + 3600, "own456", "bob"), Ok(false));
    let here: Vec<Info> = list().into_iter().filter(|i| i.portal == portal).collect();
    assert!(here.len() == 1 && here[0].username == "ana" && here[0].renews, "{here:?}");
    assert_eq!(authorization(url).as_deref(), Some("Bearer users-token"));
    sign_out(Some(portal));

    // An adopted token for an organization's own address never out-ranks the user's live sign-in
    // covering the same services, though its portal's address is longer.
    let org = "https://adoptrank.maps.arcgis.com";
    let url = "https://services1.arcgis.com/rank42/arcgis/rest/services/A/FeatureServer/0";
    sign_in("https://ar.example.org", "client", grant("x", now_secs() + 3600), vec![Scope::Org("rank42".into())]);
    assert_eq!(adopt(org, "plugin-token", now_secs() + 3600, "rank42", "bob"), Ok(true));
    assert_eq!(authorization(url).as_deref(), Some("Bearer x"));
    // Once the user's has ended (expired, nothing to renew it), the adopted one is sent.
    sign_in(
        "https://ar.example.org",
        "client",
        Grant { refresh_token: None, ..grant("x", now_secs() - 1) },
        vec![Scope::Org("rank42".into())],
    );
    assert_eq!(authorization(url).as_deref(), Some("Bearer plugin-token"));
    // The user's own sign-in to the same portal replaces the adopted one.
    sign_in(org, "client", grant("mine", now_secs() + 3600), scopes_for(org, Some("rank42"), &[]));
    let here: Vec<Info> = list().into_iter().filter(|i| i.portal == org).collect();
    assert!(here.len() == 1 && here[0].renews, "{here:?}");
    sign_out(Some(org));
    sign_out(Some("https://ar.example.org"));
    // "plugin" is never taken for a user's client id.
    assert!(!valid_client_id("plugin"));
}

#[test]
fn an_expired_adopted_token_is_not_sent() {
    let _turn = adopting();
    let portal = "https://adoptold.maps.arcgis.com";
    let url = "https://services2.arcgis.com/old789/arcgis/rest/services/A/FeatureServer/0";
    assert_eq!(adopt(portal, "old-plugin-token", now_secs() - 5, "old789", ""), Ok(true));
    assert_eq!(authorization(url), None);
    assert_eq!(with_token(url), None);
    // An expiry in milliseconds (ArcGIS's generateToken gives one) is read as such.
    let soon = now_secs() + 600;
    assert_eq!(adopt(portal, "ms-plugin-token", soon * 1000, "old789", ""), Ok(true));
    assert!(list().iter().any(|i| i.portal == portal && i.expires_at == Some(soon)));
    // One far off is cut to a year.
    adopt(portal, "far-plugin-token", now_secs() + 10 * 366 * 86_400, "old789", "").unwrap();
    assert!(list().iter().any(|i| i.portal == portal && i.expires_at.is_some_and(|e| e <= now_secs() + 366 * 86_400)));
    sign_out(Some(portal));
}

#[test]
fn adopt_refuses_an_enterprise_portal_or_plain_http() {
    let _turn = adopting();
    let soon = now_secs() + 3600;
    for portal in [
        "https://gis.example.org/portal",
        "http://www.arcgis.com",
        "https://www.arcgis.com.example.org",
        "ftp://www.arcgis.com",
        // Hosts of every organization's services: their whole host would get it.
        "https://services1.arcgis.com",
        "https://tiles.arcgis.com/tiles/abc999",
        "https://basemaps.arcgis.com",
        "https://adoptport.maps.arcgis.com:8443",
    ] {
        assert!(adopt(portal, "refused-token", soon, "abc999", "").is_err(), "{portal}");
    }
    // A bad organization id or token is refused, the token never repeated.
    for org in ["", "a/b", "abc 999", "évil", &"a".repeat(65)] {
        assert!(adopt("https://adoptbad.maps.arcgis.com", "refused-token", soon, org, "").is_err(), "{org}");
    }
    // An id that is one of the hosts' own path words would cover every organization's services
    // (`services1.arcgis.com/<org>/arcgis/…`, `tiles.arcgis.com/tiles/<org>/…`); a short one is refused too.
    for org in ["arcgis", "ArcGIS", "tiles", "TILES", "rest", "services", "sharing", "home", "apps", "content", "admin", "info", "abc12"] {
        assert!(adopt("https://adoptbad.maps.arcgis.com", "refused-token", soon, org, "").is_err(), "{org}");
    }
    let e = adopt("https://adoptbad.maps.arcgis.com", "secret token\r\n", soon, "abc999", "").unwrap_err();
    assert!(!e.contains("secret"), "{e}");
    assert!(!list().iter().any(|i| i.portal.contains("adoptbad") || i.portal.contains("gis.example.org")));
    // A real organization's covers its own services only.
    let portal = "https://adoptgood.maps.arcgis.com";
    assert_eq!(adopt(portal, "good-token", soon, "abc123def", ""), Ok(true));
    assert!(authorization("https://services1.arcgis.com/abc123def/arcgis/rest/services/S/FeatureServer/0").is_some());
    assert_eq!(authorization("https://services1.arcgis.com/otherorg/arcgis/rest/services/S/FeatureServer/0"), None);
    assert_eq!(authorization("https://tiles.arcgis.com/tiles/otherorg/arcgis/rest/services/S/MapServer/tile/1/0/0"), None);
    sign_out(Some(portal));
}

#[test]
fn adopting_never_evicts_the_users_sessions() {
    let _turn = adopting();
    let portal = "https://adoptkeep.example.org/portal";
    sign_in(portal, "client", grant("keep-token", now_secs() + 3600), scopes_for(portal, None, &[]));
    add_token("https://adoptkeep.example.org/server/rest/services", "keep-server-token").unwrap();
    for n in 0..(MAX_SESSIONS + 10) {
        let p = format!("https://adoptmany{n}.maps.arcgis.com");
        assert_eq!(adopt(&p, "many-token", now_secs() + 3600, "many123", ""), Ok(true), "{p}");
    }
    // One token handed over at a time; the user's sign-in and token stay.
    let kept = list();
    assert_eq!(kept.iter().filter(|i| i.portal.contains("adoptmany")).count(), 1);
    assert!(kept.iter().any(|i| i.portal == portal && i.renews));
    assert!(kept.iter().any(|i| i.portal == "https://adoptkeep.example.org/server" && i.token));
    assert_eq!(authorization("https://adoptkeep.example.org/portal/sharing/rest/portals/self").as_deref(), Some("Bearer keep-token"));
    sign_out(Some(&format!("https://adoptmany{}.maps.arcgis.com", MAX_SESSIONS + 9)));
    sign_out(Some(portal));
    sign_out(Some("https://adoptkeep.example.org/server"));
}
