// Drive a running Tableboat through WebView2's DevTools protocol (development only).
// Start Tableboat with:  $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9222"
// Usage: node scripts/cdp.mjs <steps.json>   (steps: [{eval}|{click:[x,y],double?}|{key}|{shot:path}|{wait:ms}])
import { readFileSync, writeFileSync } from "node:fs";

const steps = JSON.parse(readFileSync(process.argv[2], "utf8"));
const targets = await (await fetch("http://127.0.0.1:9222/json")).json();
const page = targets.find((t) => t.type === "page");
if (!page) throw new Error("no page target");
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r, { once: true }));

let id = 0;
const pending = new Map();
const logs = [];
ws.addEventListener("message", (ev) => {
  const msg = JSON.parse(ev.data);
  if (msg.id && pending.has(msg.id)) {
    pending.get(msg.id)(msg);
    pending.delete(msg.id);
  } else if (msg.method === "Runtime.consoleAPICalled") {
    logs.push(`[console.${msg.params.type}] ` + msg.params.args.map((a) => a.value ?? a.description).join(" "));
  } else if (msg.method === "Runtime.exceptionThrown") {
    logs.push(`[exception] ${msg.params.exceptionDetails.exception?.description ?? msg.params.exceptionDetails.text}`);
  }
});
const send = (method, params = {}) =>
  new Promise((resolve) => {
    const n = ++id;
    pending.set(n, resolve);
    ws.send(JSON.stringify({ id: n, method, params }));
  });

await send("Runtime.enable");
await send("Page.enable");

const mouse = async (type, x, y, clickCount = 1, button = "left") =>
  send("Input.dispatchMouseEvent", { type, x, y, button, clickCount, buttons: type === "mouseReleased" ? 0 : 1 });

for (const step of steps) {
  if (step.eval) {
    const r = await send("Runtime.evaluate", { expression: step.eval, awaitPromise: true, returnByValue: true });
    console.log("eval:", JSON.stringify(r.result?.result?.value ?? r.result?.exceptionDetails?.exception?.description ?? r.result));
  } else if (step.click) {
    const [x, y] = step.click;
    const button = step.right ? "right" : "left";
    await send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
    for (let c = 1; c <= (step.double ? 2 : 1); c++) {
      await send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button, clickCount: c, buttons: button === "right" ? 2 : 1 });
      await send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button, clickCount: c, buttons: 0 });
    }
  } else if (step.key) {
    const { key, code, modifiers = 0, keyCode } = step.key;
    await send("Input.dispatchKeyEvent", { type: "keyDown", key, code, modifiers, windowsVirtualKeyCode: keyCode });
    await send("Input.dispatchKeyEvent", { type: "keyUp", key, code, modifiers, windowsVirtualKeyCode: keyCode });
  } else if (step.text) {
    await send("Input.insertText", { text: step.text });
  } else if (step.shot) {
    const r = await send("Page.captureScreenshot", { format: "png" });
    writeFileSync(step.shot, Buffer.from(r.result.data, "base64"));
    console.log("shot:", step.shot);
  } else if (step.wait) {
    await new Promise((r) => setTimeout(r, step.wait));
  }
}
if (logs.length) console.log(logs.join("\n"));
ws.close();
process.exit(0);
