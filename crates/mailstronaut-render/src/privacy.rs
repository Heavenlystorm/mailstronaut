//! Synthetic resource probes. The syscall observer lives outside the renderer.

use super::*;

#[test]
fn removes_active_subtrees_including_foreign_markup_without_losing_safe_css() {
    let input = r#"<style>table{border-collapse:collapse}td{color:navy}</style>
        <table><tr><td colspan="2">SAFE_TABLE</td></tr></table>
        <script>PRIVATE_SCRIPT</script><form action="https://example.invalid">
        PRIVATE_FORM<input type="file" onfocus="PRIVATE_HANDLER"></form>
        <svg onload="PRIVATE_SVG_HANDLER"><foreignObject><style>PRIVATE_SVG_STYLE</style>
        <div>PRIVATE_SVG</div></foreignObject></svg>
        <math><mtext>PRIVATE_MATH</mtext></math>
        <iframe srcdoc="PRIVATE_FRAME">PRIVATE_IFRAME</iframe>
        <object data="file:///PRIVATE_OBJECT">PRIVATE_OBJECT_BODY</object>
        <template>PRIVATE_TEMPLATE</template><video poster="PRIVATE_MEDIA">PRIVATE_VIDEO</video>
        <div onclick="PRIVATE_CLICK">SAFE_TEXT</div>"#;
    let output = sanitize(input, Arc::default()).expect("sanitized synthetic fixture");
    assert!(
        !output.contains("PRIVATE_"),
        "active subtrees must be removed"
    );
    assert!(output.contains("SAFE_TABLE"));
    assert!(output.contains("SAFE_TEXT"));
    assert!(output.contains("border-collapse:collapse"));
    assert!(output.contains("colspan=\"2\""));
}

#[test]
#[ignore = "run only through the project-local privacy.sh network namespace and strace observer"]
fn trace_resource_boundary() {
    let root = fixture_root();
    let port: u16 = std::fs::read_to_string(root.join("http-port"))
        .expect("private observer port")
        .trim()
        .parse()
        .expect("observer port number");
    match std::env::var("MAILSTRONAUT_RENDER_TRACE_MODE")
        .expect("explicit observer mode")
        .as_str()
    {
        "control" => observer_control(&root, port),
        "denied" => denied_matrix(&root, port),
        _ => panic!("unknown explicit observer mode"),
    }
}

fn fixture_root() -> std::path::PathBuf {
    let root = std::path::PathBuf::from(
        std::env::var_os("MAILSTRONAUT_RENDER_TRACE_ROOT").expect("private observer root"),
    )
    .canonicalize()
    .expect("existing private root");
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates")
        .parent()
        .expect("workspace")
        .join("target")
        .canonicalize()
        .expect("project target");
    assert!(root.starts_with(workspace));
    assert!(root.join("synthetic-only").is_file());
    root
}

fn observer_control(root: &std::path::Path, port: u16) {
    use std::{
        io::{Read as _, Write as _},
        net::{TcpStream, ToSocketAddrs as _},
        time::Duration,
    };
    for name in [
        "sentinel.png",
        "sentinel.css",
        "sentinel.font",
        "sentinel.html",
    ] {
        assert_ne!(
            std::fs::read(root.join(name))
                .expect("synthetic readable file")
                .len(),
            0
        );
    }
    // The probe's isolated resolver points to its own loopback DNS observer.
    // This exercises libc's actual resolver, not a hand-labelled UDP packet.
    assert!(
        ("mailstronaut-control.invalid", 80)
            .to_socket_addrs()
            .is_err()
    );
    let mut stream = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
        .expect("HTTP observation control");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("bounded read");
    stream
        .write_all(b"GET /control HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .expect("control request");
    let mut response = String::new();
    stream
        .take(4096)
        .read_to_string(&mut response)
        .expect("bounded response");
    assert!(response.contains("302"));
    println!("Real DNS, TCP/redirect response and synthetic file observer controls exercised.");
}

fn denied_matrix(root: &std::path::Path, port: u16) {
    let urls = [
        format!("http://127.0.0.1:{port}/redirect"),
        format!("http://localhost:{port}/tracking"),
        "https://mailstronaut-tracker.invalid/tracking".into(),
        "http://10.0.0.1/private".into(),
        "http://[::1]/tracking".into(),
        format!("file://{}", root.join("sentinel.png").display()),
        "data:text/html,%3Cscript%3Ealert(1)%3C/script%3E".into(),
        "data:image/svg+xml,%3Csvg%20onload=%22alert(1)%22%3E%3C/svg%3E".into(),
        format!(
            "data:image/png;base64,{}",
            include_str!("fixtures/inline-png.base64")
        ),
        "blob:https://mailstronaut-tracker.invalid/retired".into(),
        "ftp://mailstronaut-tracker.invalid/image".into(),
        "mailstronaut-resource://message/retired/0".into(),
    ];
    let images = [InlineImage {
        content_id: "known-local",
        media_type: "image/png",
        bytes: include_bytes!("../../../assets/inline-fixture.png"),
    }];
    let mut blocked = 0;
    for url in &urls {
        let html = attack_fixture(root, url);
        let frame =
            render_with_images(&html, &images, 240, 200, false).expect("bounded passive document");
        blocked += frame.blocked_resources;
        assert!(
            frame.pixels.as_chunks::<4>().0.contains(&[255, 0, 0, 255]),
            "an exact known local raster must still paint while other resources are denied"
        );
    }
    assert!(
        blocked >= urls.len(),
        "CSS/resource fixtures must exercise the denial provider"
    );
    println!(
        "{} synthetic resource variants rendered with {blocked} denied callbacks and valid local raster pixels.",
        urls.len()
    );
}

fn attack_fixture(root: &std::path::Path, url: &str) -> String {
    let css = root.join("sentinel.css");
    let font = root.join("sentinel.font");
    let html = root.join("sentinel.html");
    format!(
        r#"
        <base href="{url}"><meta http-equiv="refresh" content="0;url={url}">
        <link rel="dns-prefetch" href="{url}"><link rel="preconnect" href="{url}">
        <link rel="stylesheet" href="{url}"><link rel="prefetch" href="{url}">
        <style>@import url('{url}');@import url('file://{css}');
        @font-face{{font-family:mailstronaut-private;src:url('file://{font}')}}
        body{{margin:0;font-family:mailstronaut-private,sans-serif}}
        .attack{{width:40px;height:40px;background-image:url('{url}');
        mask-image:url('{url}');cursor:url('{url}'),auto}}
        li{{list-style-image:url('{url}')}}</style>
        <img src="cid:known-local" width="96" height="32">
        <img src="{url}" srcset="{url} 1x" onload="fetch('{url}')" onerror="fetch('{url}')">
        <img src="cid:unknown"><div class="attack">Safe text</div><ul><li>Safe item</li></ul>
        <script>fetch('{url}');document.location='{url}';</script>
        <iframe src="file://{html}" srcdoc="&lt;img src='{url}'&gt;"></iframe>
        <form action="{url}"><input type="file"><button>Submit</button></form>
        <object data="{url}"></object><embed src="{url}">
        <svg onload="fetch('{url}')"><image href="{url}"/></svg>
        <video src="{url}" poster="{url}"></video><audio src="{url}"></audio>
        <a href="{url}" ping="{url}">An inert link</a>
    "#,
        css = css.display(),
        font = font.display(),
        html = html.display()
    )
}
