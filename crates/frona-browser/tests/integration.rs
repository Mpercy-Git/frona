use std::time::Duration;

use frona_browser::{BrowserConnection, ElementTarget, ExtractFormat};

fn ws_url() -> Option<String> {
    std::env::var("FRONA_TEST_BROWSER_WS_URL").ok()
}

async fn connect() -> BrowserConnection {
    let url = ws_url().expect("set FRONA_TEST_BROWSER_WS_URL to a browserless ws:// endpoint");
    BrowserConnection::connect(&url, Duration::from_secs(30))
        .await
        .expect("connect to browserless")
}

fn data_url(html: &str) -> String {
    let encoded = html
        .replace('#', "%23")
        .replace(' ', "%20")
        .replace('\n', "%0A");
    format!("data:text/html,{encoded}")
}

const SIMPLE_PAGE: &str = r#"<!doctype html><html><head><title>T</title></head><body>
<h1>Sign in</h1>
<form>
  <label>Email <input type="text" id="email"></label>
  <label>Password <input type="password" id="pw"></label>
  <button id="go">Sign in</button>
</form>
<a href="/forgot">Forgot password?</a>
</body></html>"#;

#[tokio::test]
#[ignore = "requires FRONA_TEST_BROWSER_WS_URL"]
async fn navigate_returns_title() {
    let conn = connect().await;
    conn.navigate(&data_url(SIMPLE_PAGE), true).await.unwrap();
    let snap = conn.snapshot(false, false).await.unwrap();
    assert!(
        snap.tree.contains("Sign in"),
        "snapshot missing heading: {}",
        snap.tree
    );
    assert!(
        snap.tree.contains("button"),
        "snapshot missing button role: {}",
        snap.tree
    );
    assert!(
        snap.tree.contains("Forgot password"),
        "snapshot missing link: {}",
        snap.tree
    );
    conn.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires FRONA_TEST_BROWSER_WS_URL"]
async fn extract_returns_body_text() {
    let conn = connect().await;
    conn.navigate(&data_url(SIMPLE_PAGE), true).await.unwrap();
    let text = conn
        .extract(Some("body"), ExtractFormat::Text)
        .await
        .unwrap();
    assert!(text.contains("Sign in"));
    conn.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires FRONA_TEST_BROWSER_WS_URL"]
async fn evaluate_returns_value() {
    let conn = connect().await;
    conn.navigate(&data_url(SIMPLE_PAGE), true).await.unwrap();
    let v = conn.evaluate("2 + 2", false).await.unwrap();
    assert_eq!(v.as_i64(), Some(4));
    conn.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires FRONA_TEST_BROWSER_WS_URL"]
async fn click_by_selector_works() {
    let html = r#"<!doctype html><html><body>
        <button id="b" onclick="document.title = 'clicked'">Press</button>
        </body></html>"#;
    let conn = connect().await;
    conn.navigate(&data_url(html), true).await.unwrap();
    conn.click(ElementTarget::Selector("#b")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let title = conn
        .evaluate("document.title", false)
        .await
        .unwrap();
    assert_eq!(title.as_str(), Some("clicked"));
    conn.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires FRONA_TEST_BROWSER_WS_URL"]
async fn snapshot_index_resolves_to_element() {
    let html = r#"<!doctype html><html><body>
        <button id="b1" onclick="document.title='one'">First</button>
        <button id="b2" onclick="document.title='two'">Second</button>
        </body></html>"#;
    let conn = connect().await;
    conn.navigate(&data_url(html), true).await.unwrap();
    let snap = conn.snapshot(false, false).await.unwrap();
    // Find the index for "Second"
    let mut second_index: Option<usize> = None;
    for line in snap.tree.lines() {
        if line.contains("Second")
            && let Some(idx_part) = line.split("[index=").nth(1)
            && let Some(idx_str) = idx_part.split(']').next()
            && let Ok(i) = idx_str.parse::<usize>()
        {
            second_index = Some(i);
            break;
        }
    }
    let idx = second_index.expect("Second button has no index in snapshot");
    conn.click(ElementTarget::Index(idx)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let title = conn.evaluate("document.title", false).await.unwrap();
    assert_eq!(title.as_str(), Some("two"));
    conn.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires FRONA_TEST_BROWSER_WS_URL"]
async fn wait_for_selector_waits() {
    let html = r#"<!doctype html><html><body><div id="root"></div>
        <script>setTimeout(()=>{
            const e=document.createElement('div');e.id='late';e.textContent='here';
            document.body.appendChild(e);
        }, 300);</script>
        </body></html>"#;
    let conn = connect().await;
    conn.navigate(&data_url(html), true).await.unwrap();
    conn.wait_for_selector("#late", Duration::from_secs(3))
        .await
        .unwrap();
    conn.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires FRONA_TEST_BROWSER_WS_URL"]
async fn select_changes_dropdown_value() {
    let html = r#"<!doctype html><html><body>
        <select id="s" onchange="document.title=this.value">
            <option value="a">Alpha</option>
            <option value="b">Bravo</option>
            <option value="c">Charlie</option>
        </select></body></html>"#;
    let conn = connect().await;
    conn.navigate(&data_url(html), true).await.unwrap();
    conn.select(ElementTarget::Selector("#s"), "b").await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let title = conn.evaluate("document.title", false).await.unwrap();
    assert_eq!(title.as_str(), Some("b"));
    conn.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires FRONA_TEST_BROWSER_WS_URL"]
async fn select_matches_by_visible_text_too() {
    let html = r#"<!doctype html><html><body>
        <select id="s" onchange="document.title=this.value">
            <option value="x1">First</option>
            <option value="x2">Second</option>
        </select></body></html>"#;
    let conn = connect().await;
    conn.navigate(&data_url(html), true).await.unwrap();
    conn.select(ElementTarget::Selector("#s"), "Second").await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let title = conn.evaluate("document.title", false).await.unwrap();
    assert_eq!(title.as_str(), Some("x2"));
    conn.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires FRONA_TEST_BROWSER_WS_URL"]
async fn scroll_moves_page() {
    let html = r#"<!doctype html><html><body style="height:5000px">
        <div style="height:5000px">spacer</div></body></html>"#;
    let conn = connect().await;
    conn.navigate(&data_url(html), true).await.unwrap();
    let before = conn
        .evaluate("window.scrollY", false)
        .await
        .unwrap()
        .as_f64()
        .unwrap_or(0.0);
    assert_eq!(before, 0.0);

    conn.scroll(Some(800)).await.unwrap();
    let after_relative = conn
        .evaluate("window.scrollY", false)
        .await
        .unwrap()
        .as_f64()
        .unwrap_or(0.0);
    assert!(after_relative > 700.0, "expected scrollY > 700, got {after_relative}");

    conn.scroll(None).await.unwrap();
    let after_bottom = conn
        .evaluate("window.scrollY", false)
        .await
        .unwrap()
        .as_f64()
        .unwrap_or(0.0);
    assert!(
        after_bottom > after_relative,
        "scroll-to-bottom should land below the explicit offset (was {after_relative}, now {after_bottom})"
    );
    conn.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires FRONA_TEST_BROWSER_WS_URL"]
async fn hover_dispatches_mouseover() {
    let html = r#"<!doctype html><html><body>
        <button id="b" onmouseover="document.title='hovered'">Hover me</button>
        </body></html>"#;
    let conn = connect().await;
    conn.navigate(&data_url(html), true).await.unwrap();
    conn.hover(ElementTarget::Selector("#b")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let title = conn.evaluate("document.title", false).await.unwrap();
    assert_eq!(title.as_str(), Some("hovered"));
    conn.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires FRONA_TEST_BROWSER_WS_URL"]
async fn connection_stays_alive_across_keepalive_interval() {
    // Default keepalive ticks every 25s. Idle the connection just over that
    // interval; a healthy keepalive prevents the WebSocket from going stale.
    // We then verify the connection is still usable for normal commands.
    let conn = connect().await;
    conn.navigate(&data_url("<html><title>ok</title></html>"), true)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(30)).await;
    let v = conn.evaluate("1 + 1", false).await.unwrap();
    assert_eq!(v.as_i64(), Some(2));
    conn.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires FRONA_TEST_BROWSER_WS_URL"]
async fn compact_snapshot_is_smaller_than_full() {
    let html = r##"<!doctype html><html><body>
        <header><nav><ul>
            <li><a href="#a">Alpha</a></li>
            <li><a href="#b">Bravo</a></li>
        </ul></nav></header>
        <main><article>
            <h1>Title</h1>
            <p>Some prose that should not survive compact mode.</p>
            <p>More prose without interactive elements.</p>
            <p>Even more padding to make the full tree clearly larger.</p>
            <button id="x">Action</button>
        </article></main>
        <footer><p>Footer copy without interactives.</p></footer>
    </body></html>"##;
    let conn = connect().await;
    conn.navigate(&data_url(html), true).await.unwrap();

    let full = conn.snapshot(false, false).await.unwrap();
    let compact = conn.snapshot(false, true).await.unwrap();

    assert!(
        compact.tree.len() < full.tree.len(),
        "compact ({} B) should be smaller than full ({} B):\n--- full ---\n{}\n--- compact ---\n{}",
        compact.tree.len(),
        full.tree.len(),
        full.tree,
        compact.tree
    );
    assert!(
        compact.tree.contains("Action"),
        "compact must keep the actionable button: {}",
        compact.tree
    );
    assert!(
        compact.tree.contains("Alpha") && compact.tree.contains("Bravo"),
        "compact must keep the actionable links: {}",
        compact.tree
    );
    // Lines with no `[index=` and no inline `: "value"` (e.g. unlabeled
    // wrappers / pure-container nodes) should be stripped in compact mode.
    // Note: paragraphs with text are kept — they have `: <text>` markers and
    // an LLM may want to read prose context. The win here is dropping
    // structure-only wrappers.
    let full_lines = full.tree.lines().count();
    let compact_lines = compact.tree.lines().count();
    assert!(
        compact_lines < full_lines,
        "compact must drop at least one inert wrapper line (full={full_lines}, compact={compact_lines})"
    );
    conn.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires FRONA_TEST_BROWSER_WS_URL"]
async fn snapshot_diff_emits_change_only() {
    let html = r#"<!doctype html><html><body>
        <div id="root"><button>One</button></div>
        </body></html>"#;
    let conn = connect().await;
    conn.navigate(&data_url(html), true).await.unwrap();
    let first = conn.snapshot(false, false).await.unwrap();
    assert!(first.tree.contains("One"));

    conn.evaluate(
        "document.getElementById('root').innerHTML = '<button>One</button><button>Two</button>'",
        false,
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;

    let diff = conn.snapshot(true, false).await.unwrap();
    assert!(
        diff.tree
            .lines()
            .any(|l| l.starts_with("+ ") && l.contains("Two")),
        "expected an added line for Two, got:\n{}",
        diff.tree
    );
    conn.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires FRONA_TEST_BROWSER_WS_URL"]
async fn ref_survives_dom_mutation() {
    // Simulates a React-style re-render: same logical button, but the DOM
    // node is replaced (different backend_node_id) between snapshots.
    let html = r#"<!doctype html><html><body>
        <div id="root"></div>
        <script>
            function render(text) {
                document.getElementById('root').innerHTML = '';
                const b = document.createElement('button');
                b.textContent = text;
                b.onclick = () => { document.title = 'clicked-' + text; };
                document.getElementById('root').appendChild(b);
            }
            render('Submit');
        </script>
        </body></html>"#;
    let conn = connect().await;
    conn.navigate(&data_url(html), true).await.unwrap();

    let snap = conn.snapshot(false, false).await.unwrap();
    let idx = snap
        .tree
        .lines()
        .find(|l| l.contains("Submit"))
        .and_then(|l| l.split("[index=").nth(1))
        .and_then(|l| l.split(']').next())
        .and_then(|s| s.parse::<usize>().ok())
        .expect("Submit button has an index in snapshot");

    // Re-render to invalidate the original backend_node_id without changing
    // the button's logical role/name. (Same name, fresh DOM node.)
    conn.evaluate("render('Submit')", false).await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Click via the *old* snapshot ref — must re-resolve via role+name+nth.
    conn.click(ElementTarget::Index(idx)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let title = conn.evaluate("document.title", false).await.unwrap();
    assert_eq!(title.as_str(), Some("clicked-Submit"));
    conn.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires FRONA_TEST_BROWSER_WS_URL"]
async fn tabs_lifecycle() {
    let conn = connect().await;
    conn.navigate(&data_url("<html><title>a</title></html>"), true)
        .await
        .unwrap();
    let t1 = conn.tabs().await.unwrap();
    let n1 = t1.len();
    conn.new_tab(&data_url("<html><title>b</title></html>"))
        .await
        .unwrap();
    let t2 = conn.tabs().await.unwrap();
    assert!(t2.len() > n1, "expected one more tab after new_tab");
    conn.close_active_tab().await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let t3 = conn.tabs().await.unwrap();
    assert!(
        t3.len() < t2.len(),
        "expected fewer tabs after close ({} -> {})",
        t2.len(),
        t3.len()
    );
    conn.disconnect().await.unwrap();
}
