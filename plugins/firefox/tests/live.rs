//! Run explicitly outside restricted sandboxes. Only a newly created profile is used.
use serde_json::json;
use std::{
    fs,
    net::TcpListener,
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use top_dog_core::monitor;
use top_dog_firefox::{Firefox, marionette::Client};
use top_dog_protocol::{Operation, Reply};

struct Browser {
    child: Child,
    profile: PathBuf,
}
impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.profile);
    }
}

#[test]
#[ignore = "requires Firefox with loopback sockets; uses only a disposable profile"]
fn maps_live_pid_selects_existing_tab_and_rejects_closed_target() {
    let binary = std::env::var("TOP_DOG_FIREFOX_BINARY").unwrap_or_else(|_| "firefox".into());
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback sockets are required");
    let address = listener.local_addr().unwrap();
    drop(listener);
    let profile = std::env::temp_dir().join(format!(
        "top-dog-firefox-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&profile).unwrap();
    fs::write(profile.join("user.js"),format!("user_pref(\"marionette.port\",{});\nuser_pref(\"marionette.force-local\",true);\nuser_pref(\"browser.shell.checkDefaultBrowser\",false);\nuser_pref(\"browser.startup.page\",0);\n",address.port())).unwrap();
    let child = Command::new(binary)
        .args([
            "--headless",
            "--no-remote",
            "--marionette",
            "--remote-allow-system-access",
            "--profile",
        ])
        .arg(&profile)
        .arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("set TOP_DOG_FIREFOX_BINARY to a runnable Firefox executable");
    let mut browser = Browser { child, profile };
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut client = loop {
        if let Some(status) = browser.child.try_wait().unwrap() {
            panic!("Firefox exited before connecting: {status}");
        }
        match Client::connect(address) {
            Ok(client) => break client,
            Err(e) if Instant::now() < deadline => {
                let _ = e;
                thread::sleep(Duration::from_millis(100));
            }
            Err(e) => panic!("cannot connect: {e}"),
        }
    };
    let create = "const win=Services.wm.getMostRecentWindow('navigator:browser');for(const title of ['TopDogOne','TopDogTwo'])win.gBrowser.addTab('data:text/html,<title>'+title+'</title>',{triggeringPrincipal:Services.scriptSecurityManager.getSystemPrincipal()});return true;";
    client.execute(create, json!({})).unwrap();
    let discover = "const w=Services.wm.getMostRecentWindow('navigator:browser');return Array.from(w.gBrowser.tabs).map(t=>({title:t.linkedBrowser.contentTitle,pid:t.linkedBrowser.browsingContext.currentWindowGlobal?.osPid})).filter(t=>t.title==='TopDogTwo'&&t.pid>0);";
    let deadline = Instant::now() + Duration::from_secs(10);
    let pid = loop {
        let tabs = client.execute(discover, json!({})).unwrap();
        if let Some(pid) = tabs[0]["pid"].as_i64() {
            break pid as i32;
        }
        assert!(Instant::now() < deadline, "test tab did not load");
        thread::sleep(Duration::from_millis(100));
    };
    let count = "return Array.from(Services.wm.getEnumerator('navigator:browser')).reduce((n,w)=>n+w.gBrowser.tabs.length,0);";
    let before = client.execute(count, json!({})).unwrap();
    drop(client);
    let plugin = Firefox {
        endpoints: vec![address],
        include_private: false,
    };
    let process = monitor::identity(pid).unwrap();
    let Reply::Resolution(resolution) = plugin.handle(Operation::Resolve { process }).unwrap()
    else {
        panic!("wrong reply");
    };
    let target = resolution
        .targets
        .iter()
        .find(|t| t.title == "TopDogTwo")
        .expect("PID must map to the test tab");
    let Reply::Activation(activation) = plugin
        .handle(Operation::Activate {
            process,
            target_id: target.id.clone(),
        })
        .unwrap()
    else {
        panic!("wrong reply");
    };
    assert!(activation.selected);
    let mut client = Client::connect(address).unwrap();
    assert_eq!(
        before,
        client.execute(count, json!({})).unwrap(),
        "activation must not create tabs"
    );
    assert_eq!(client.execute("return Services.wm.getMostRecentWindow('navigator:browser').gBrowser.selectedTab.linkedBrowser.contentTitle;",json!({})).unwrap(),"TopDogTwo");
    client.execute("const w=Services.wm.getMostRecentWindow('navigator:browser');w.gBrowser.removeTab(w.gBrowser.selectedTab);return true;",json!({})).unwrap();
    drop(client);
    assert!(
        plugin
            .handle(Operation::Activate {
                process,
                target_id: target.id.clone()
            })
            .is_err()
    );
}
