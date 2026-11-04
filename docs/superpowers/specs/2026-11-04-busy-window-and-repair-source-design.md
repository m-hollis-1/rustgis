# Busy window, Repair Source helper and one ArcGIS Online sign-in

Date: 2026-11-04. Follows desktop testing of the ArcGIS Pro map import (Part 3).

## Why

Testing a large Pro project (12 maps, 7 layouts, about 100 service layers) on the desktop
turned up three problems:

1. **Import and save look frozen.** Clicking Import in the Pro picker, or saving a large
   project, gives no sign that anything is happening for minutes. The window stops drawing and
   Windows may grey it out as "Not Responding". A user can't tell whether to wait or kill it, and
   killing it mid-save can lose work.
2. **Retry after sign-in didn't open token-secured layers.** The tester signed in through the
   ArcGIS Online plugin. The plugin is a separate process with its own sign-in (`generateToken`
   with stored credentials); RustGIS's `layer.retry` → `service.add` only sends tokens the host
   holds (`rustgis_tiles::arcgis_auth`), so it never had one. The automatic retry after sign-in
   only follows the host's own "Sign in to ArcGIS" (Add From Service), not the plugin's.
3. **Fixing a broken layer isn't discoverable.** The layer menu has Retry (services, and files
   that may have turned up) and Repair Data Source… (files only, one layer at a time), and a
   broken layer's reason is stored with it. Nothing tells the user *what* to do for *this* layer.

Also found: a self-hosted ArcGIS Server the test project uses presents a TLS certificate that
Windows doesn't trust ("issued by an authority that is not trusted"). No sign-in fixes that, and
RustGIS must not skip certificate checks. The helper has to recognise this case and say so.

## Goals

- During a long blocking operation the user is told, before it starts, what is happening, that
  it can take minutes, and not to close RustGIS.
- One ArcGIS Online sign-in — the plugin's or the built-in one — lets Retry open the
  organization's hosted layers.
- A broken layer's right-click menu offers **Repair Source…**, a dialog that says in plain words
  what's wrong and offers the fix that fits.

## Non-goals

- Live progress bars or Cancel for import/save. That needs commands to run off the UI thread, a
  large change to how the engine runs and a conflict magnet with upstream. (Tester chose the
  working window now; live progress may come later as its own project.)
- Trusting untrusted certificates, or any way to turn certificate checks off.
- Repairing databases (PostGIS) or tile archives in the new helper; their existing commands stay.

## Anti-goals

- A working window that can be dismissed, or that hides an error that happened.
- Sending a token to a host it shouldn't go to. The plugin's rule today: https only, the
  signed-in portal's host or `*.arcgis.com`. The host's scopes in `arcgis_auth` stay the authority.
- A fork change that makes upstream merges harder than needed: new code in new files where
  possible; shared files touched in small, well-marked places.

## Design

### 1. Busy window

- A modal working window, painted for at least one frame **before** the blocking call runs, so it
  is on screen while the UI thread is busy. Mechanism (builder's call on detail): the action is
  queued with its message; the next frame paints the window and requests a repaint; the frame
  after runs the action, then clears it.
- Text: what is happening ("Importing 3 maps and 2 layouts from <file name>…", "Saving <project
  name>…", "Opening <project name>…"), then "This can take several minutes for a large project.
  Windows may say RustGIS is not responding — don't close it." A spinner is drawn but won't
  animate while blocked; that's acceptable.
- Covers: the Pro picker's Import, opening/dropping a Pro file that imports without the picker,
  Save, Save As, and opening a project file. The builder may add other known-long blocking
  commands where the same queue fits in a line or two.
- Errors after the action are shown exactly as today.
- Input is ignored while the window is up, so a click queued during the block doesn't land
  afterwards.

### 2. Repair Source helper

Right-click a broken layer → **Repair Source…**. For broken layers it replaces the separate Retry
and Repair Data Source… entries; healthy layers keep today's entries. The dialog states the layer,
where its data was, and the reason, then a diagnosis and its actions:

| Diagnosis | Shown to the user | Actions |
|---|---|---|
| Needs sign-in (token required, 498/499, 401/403 from an ArcGIS service) | "This layer is on <server>, which needs you to sign in." | **Sign in to ArcGIS Online** (the built-in sign-in; when it completes, this and every other broken layer is retried), Retry, Remove |
| Certificate not trusted | "<server>'s security certificate isn't trusted on this computer, so RustGIS can't connect safely. Ask whoever runs that server, or check you're on the network it's meant for." | Retry, Remove |
| Service didn't answer / not there | "<server> didn't answer" / "the service isn't there any more" | Retry, **Use another address…** (a service URL; the layer and its style kept), Remove |
| File not found | "The file isn't at <old path>." | **Browse for the file…**, then: "N other broken layers had data in the same folder; M of them are in the new folder. Fix them too?" Yes/No. Remove |
| Anything else | the reason, verbatim | Retry, Remove |

- Prefer classifying from what the open attempt returns (status codes, error kinds) over
  matching message text, where the code allows.
- "Fix them too" repairs each sibling through `layer.repairSource`, skipping (and listing) any
  whose file isn't in the new folder or whose fields don't fit.
- An action that fixes the layer closes the dialog with a notice; a failed action keeps the dialog
  open with the new reason.
- The dialog is reachable as a command (`ui.dialog`), as everything in RustGIS is.

### 3. One ArcGIS Online sign-in (plugin repo, plus a small host change if needed)

- When the plugin signs in or renews its token, it hands the token to RustGIS for the hosts it
  already trusts with it, so `service.add` and Retry send it there. Preferred: the existing
  `arcgis.token` command (plugins may call it) for each `*.arcgis.com` server root that a layer
  in the open map uses. If that can't cover it cleanly (`arcgis.token` sessions never expire
  host-side while the plugin's tokens last 120 minutes), the builder may add a small host command
  that takes a token with its expiry and portal, scoped as a built-in ArcGIS Online sign-in is.
- After handing it over, the plugin runs `layer.retry` with `all: true` when any layer is broken,
  and says how many opened.
- A self-hosted server that accepts ArcGIS Online logins: the tester reports the company server
  takes the same login. It is unreachable from this machine until its certificate problem is
  fixed, so this part can't be verified now. Ship the `*.arcgis.com` part; leave the self-hosted
  case as a noted follow-on.
- Plugin version bump (delivery refuses a plugin commit without one).

## Constraints

- AGENTS.md: no unwrap/expect/panic/direct indexing outside tests; crates under apps/ build for
  wasm32 (the busy window and helper must compile there; sign-in follows today's web behaviour).
- Privacy rule: no company, client, people or G: drive names in code, tests, fixtures, docs or
  commit messages. The certificate finding is described generically.
- Tokens are never written to the project or logs (existing rules in `arcgis_auth` and
  `sources.rs`).

## Decisions settled after the builder check (Claude's calls, not the tester's)

1. **Token handoff uses a new host command.** Plugins can't see layers' service addresses
   (`host.layers` gives ids, names, fields and counts), and `arcgis.token` sessions never expire.
   So add `arcgis.adoptToken {"portal", "token", "expires", "org"}`, which keeps a session scoped
   as a built-in ArcGIS Online sign-in is (`scopes_for`: the portal, `www.arcgis.com`'s
   `sharing/rest`, and the organization's hosted services by its id), and expires with the
   token. It is refused for anything but an ArcGIS Online portal on https. The plugin then runs
   `layer.retry {"all": true}` itself when the map has broken layers. Plugin: `minRustgisVersion`
   rises to the host release that carries the command, and the plugin carries on without the
   handoff (with a notice once) on an older host.
2. **When the plugin hands over:** on its Sign In command, and on any command where it fetched a
   new token. A host session that has gone stale is acceptable: Retry then gets "needs sign-in",
   and the helper offers signing in.
3. **The diagnosis comes from the stored reason text**, classified against a fixed set of phrases
   (token required, 498/499/401/403, certificate and trust wording including Windows', not
   found/404, no answer/timeouts, file not found), plus a structured kind kept in memory (not in
   the project) when a retry in this session returns one. No probe when the dialog opens.
4. **The busy window is a UI matter only.** Menus, buttons, drop, recent files and command search
   queue the call; `app.run` and the commands themselves (as tests, plugins and scripts call
   them) stay synchronous and unchanged.
5. **Scope:** Repair Source… covers feature layers whose `source.is_broken()`. Broken rasters and
   tile archives keep their own entries. "Use another address…" gets a new undoable command that
   points a broken service layer at another service URL, as `layer.retry` does for its own.

## Left to the builder

- Exact dialog layout and wording beyond the above; reuse existing dialog helpers.
- How the busy queue is stored on the app and which tests pin it (kittest UI tests exist for the
  Pro picker in `mod aprx_tests`).
- Whether Repair Source… is also linked from the broken layer's tooltip in Contents.
