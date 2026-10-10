// Reports the focused window's active tab (hostname only, never the path) and whether it
// is playing audio to screentimed on localhost. Incognito tabs are never reported.

const ENDPOINT = "http://127.0.0.1:47821/api/web";

function browserFamily() {
  const ua = navigator.userAgent;
  if (navigator.brave) return "brave";
  if (ua.includes("Edg/")) return "edge";
  if (ua.includes("OPR/")) return "opera";
  if (ua.includes("Vivaldi")) return "vivaldi";
  if (ua.includes("Firefox/")) return "firefox";
  return "chrome";
}

let last = "";

async function report(force = false) {
  let domain = null;
  let audible = false;
  try {
    const [tab] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
    if (tab && !tab.incognito && tab.url) {
      const url = new URL(tab.url);
      if (url.protocol === "http:" || url.protocol === "https:") domain = url.hostname;
      audible = Boolean(tab.audible);
    }
  } catch {
    // No focused window (all minimized): report nothing.
  }
  const body = JSON.stringify({ browser: browserFamily(), domain, audible });
  // Re-send unchanged state only on the heartbeat, so the daemon can tell we are alive.
  if (body === last && !force) return;
  last = body;
  try {
    await fetch(ENDPOINT, { method: "POST", headers: { "content-type": "application/json", "x-screentime": "1" }, body });
  } catch {
    // screentimed not running; try again on the next change.
  }
}

chrome.tabs.onActivated.addListener(() => report());
chrome.tabs.onUpdated.addListener((_id, change, tab) => {
  if (tab.active && (change.url || change.audible !== undefined || change.status === "complete")) report();
});
chrome.windows.onFocusChanged.addListener(() => report());
chrome.alarms.create("heartbeat", { periodInMinutes: 0.5 });
chrome.alarms.onAlarm.addListener(() => report(true));
report();
