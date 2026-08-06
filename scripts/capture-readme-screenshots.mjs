import { spawn } from "node:child_process";
import { mkdir, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";

const edgePath = "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe";
const outputDirectory = path.resolve("docs", "images");
const profileDirectory = path.join(os.tmpdir(), `codex-installer-readme-${Date.now()}`);
const debuggingPort = 9223;

const delay = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

async function waitForPage() {
  for (let attempt = 0; attempt < 60; attempt += 1) {
    try {
      const response = await fetch(`http://127.0.0.1:${debuggingPort}/json`);
      const pages = await response.json();
      const page = pages.find((item) => item.type === "page");
      if (page?.webSocketDebuggerUrl) return page;
    } catch {
      // Edge is still starting.
    }
    await delay(250);
  }
  throw new Error("Timed out while waiting for the documentation browser");
}

function createCdpClient(url) {
  const socket = new WebSocket(url);
  const pending = new Map();
  let nextId = 0;

  socket.addEventListener("message", ({ data }) => {
    const message = JSON.parse(data);
    if (!message.id || !pending.has(message.id)) return;
    const { resolve, reject } = pending.get(message.id);
    pending.delete(message.id);
    if (message.error) reject(new Error(message.error.message));
    else resolve(message.result);
  });

  return {
    ready: new Promise((resolve, reject) => {
      socket.addEventListener("open", resolve, { once: true });
      socket.addEventListener("error", reject, { once: true });
    }),
    send(method, params = {}) {
      const id = ++nextId;
      socket.send(JSON.stringify({ id, method, params }));
      return new Promise((resolve, reject) => pending.set(id, { resolve, reject }));
    },
    close() {
      socket.close();
    },
  };
}

async function capture(client, name) {
  const { data } = await client.send("Page.captureScreenshot", {
    format: "png",
    fromSurface: true,
    captureBeyondViewport: false,
  });
  await writeFile(path.join(outputDirectory, name), Buffer.from(data, "base64"));
}

async function evaluate(client, expression) {
  const result = await client.send("Runtime.evaluate", {
    expression,
    awaitPromise: true,
    returnByValue: true,
  });
  if (result.exceptionDetails) throw new Error(result.exceptionDetails.text);
  return result.result?.value;
}

await mkdir(outputDirectory, { recursive: true });
const browser = spawn(
  edgePath,
  [
    "--headless=new",
    "--disable-gpu",
    "--hide-scrollbars",
    "--no-first-run",
    `--remote-debugging-port=${debuggingPort}`,
    `--user-data-dir=${profileDirectory}`,
    "--window-size=1280,900",
    "http://localhost:5173",
  ],
  { stdio: "ignore" },
);

try {
  const page = await waitForPage();
  const client = createCdpClient(page.webSocketDebuggerUrl);
  await client.ready;
  await client.send("Page.enable");
  await client.send("Runtime.enable");
  await client.send("Emulation.setDeviceMetricsOverride", {
    width: 1280,
    height: 900,
    deviceScaleFactor: 1,
    mobile: false,
  });
  await delay(1200);
  await evaluate(client, `document.fonts.ready.then(() => {
    document.querySelector('.test-mode-note')?.remove();
    return true;
  })`);
  await capture(client, "app-choose.png");

  const started = await evaluate(client, `(() => {
    const button = [...document.querySelectorAll('button')]
      .find((item) => item.textContent.includes('开始安装'));
    button?.click();
    return Boolean(button);
  })()`);
  if (!started) throw new Error("Could not find the start installation button");
  await delay(1500);
  await capture(client, "app-installing.png");

  await delay(3500);
  await evaluate(client, `(() => {
    const items = document.querySelectorAll('.completion-meta span');
    if (items[0]?.lastChild) items[0].lastChild.textContent = ' Desktop（Microsoft Store / MSIX）';
    if (items[2]?.lastChild) items[2].lastChild.textContent = ' 已完成环境检测';
    return document.querySelector('.complete-screen') !== null;
  })()`);
  await capture(client, "app-complete.png");
  client.close();
} finally {
  browser.kill();
  await delay(500);
  await rm(profileDirectory, { recursive: true, force: true });
}
