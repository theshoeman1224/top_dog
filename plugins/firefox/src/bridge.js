// Executed in a fresh Marionette chrome sandbox. No page script is evaluated.
// PID lookup follows Firefox's about:processes BrowsingContext tree approach.
const options = arguments[0];
const { PrivateBrowsingUtils } = ChromeUtils.importESModule(
  "resource://gre/modules/PrivateBrowsingUtils.sys.mjs"
);
const applicationPid = Services.appinfo.processID;
const browserVersion = Services.appinfo.version;
if (options.action === "diagnose") {
  return { applicationPid, browserVersion };
}

function walk(context, root, found) {
  if (!context) return;
  const global = context.currentWindowGlobal;
  if (global && global.osPid === options.pid) {
    found.push({
      role: context === root ? "main_page" : "embedded_frame",
      context: String(context.id),
      document: String(global.innerWindowId),
    });
  }
  for (const child of context.children) walk(child, root, found);
}

const targets = [];
let activation = null;
for (const win of Services.wm.getEnumerator("navigator:browser")) {
  if (!win.gBrowser) continue;
  if (!options.includePrivate && PrivateBrowsingUtils.isWindowPrivate(win)) continue;
  for (const tab of win.gBrowser.tabs) {
    if (tab.closing) continue;
    const browser = tab.linkedBrowser;
    const root = browser.browsingContext;
    if (!root) continue;
    const matches = [];
    walk(root, root, matches);
    if (matches.length === 0) continue;
    const token = JSON.stringify({
      applicationPid,
      browser: String(browser.browserId),
      root: String(root.id),
      matches,
    });
    if ((options.action === "activate" || options.action === "focus") && token === options.target) {
      if (options.action === "activate") win.gBrowser.selectedTab = tab;
      let focusRequested = false;
      if (options.action === "focus" && win.gBrowser.selectedTab === tab) {
        try { win.focus(); focusRequested = true; } catch (_) { /* Selection still succeeded. */ }
      }
      activation = { selected: win.gBrowser.selectedTab === tab, focusRequested };
    }
    targets.push({
      id: token,
      title: browser.contentTitle || tab.label || "Untitled tab",
      url: browser.currentURI ? browser.currentURI.spec : null,
      window: String(win.browsingContext.id),
      relationship: matches.some(m => m.role === "main_page") ? "main_page" : "embedded_frame",
      can_activate: true,
    });
  }
}
return { applicationPid, browserVersion, targets, activation };
