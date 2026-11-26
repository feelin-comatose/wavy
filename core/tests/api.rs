//! Addon API over a real socket: token check, state, commands (docs/addons.md).
use serde_json::{json, Value};
use wavy_core::overlay::OverlayServer;

#[test]
fn api_needs_token_and_forwards_commands() {
    let srv = OverlayServer::new();
    let port = 39000 + (std::process::id() % 1000) as u16;
    srv.start(port).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let woken = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let w2 = woken.clone();
    srv.set_api("secret-token-0123456789abcdef0123".into(), tx, std::sync::Arc::new(move || w2.store(true, std::sync::atomic::Ordering::SeqCst)));
    srv.set_api_state(json!({"track": {"title": "Kyoto at 3 AM"}, "playing": true}));
    let base = format!("http://127.0.0.1:{port}/api/v1");
    let agent = ureq::Agent::new_with_config(ureq::Agent::config_builder().http_status_as_error(false).build());

    // no token / wrong token → 401
    assert_eq!(agent.get(&format!("{base}/state")).call().unwrap().status(), 401);
    assert_eq!(agent.get(&format!("{base}/state")).header("Authorization", "Bearer nope").call().unwrap().status(), 401);

    // header token → state
    let mut r = agent.get(&format!("{base}/state")).header("Authorization", "Bearer secret-token-0123456789abcdef0123").call().unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = serde_json::from_str(&r.body_mut().read_to_string().unwrap()).unwrap();
    assert_eq!(v["track"]["title"], "Kyoto at 3 AM");

    // query token works too (for EventSource)
    assert_eq!(agent.get(&format!("{base}/ping?token=secret-token-0123456789abcdef0123")).call().unwrap().status(), 200);

    // command reaches the app
    let r = agent.post(&format!("{base}/command")).header("Authorization", "Bearer secret-token-0123456789abcdef0123")
        .header("Content-Type", "application/json").send(r#"{"cmd":"seek","value":42}"#).unwrap();
    assert_eq!(r.status(), 202);
    let (cmd, body) = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
    assert_eq!((cmd.as_str(), body["value"].as_f64()), ("seek", Some(42.0)));
    assert!(woken.load(std::sync::atomic::Ordering::SeqCst), "the app must be woken to run the command");

    // unknown command → 400, nothing sent
    let r = agent.post(&format!("{base}/command")).header("Authorization", "Bearer secret-token-0123456789abcdef0123").send(r#"{"cmd":"rm -rf"}"#).unwrap();
    assert_eq!(r.status(), 400);
    assert!(rx.try_recv().is_err());
}
