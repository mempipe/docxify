// Shared startup for the browser tests, profiler and Word comparison.
import { spawn } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";

export function launchChrome(profile, port, size = "1100,1400") {
  return spawn("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", [
    "--headless=new", "--no-first-run", `--user-data-dir=${profile}`,
    `--remote-debugging-port=${port}`, `--window-size=${size}`, "about:blank",
  ], { stdio: "ignore" });
}

export async function chromeEndpoint(chrome, port) {
  const deadline = Date.now() + 120_000;
  while (Date.now() < deadline) {
    if (chrome.exitCode !== null || chrome.signalCode !== null) {
      throw new Error(`Chrome exited before its debugging port opened (${chrome.exitCode ?? chrome.signalCode})`);
    }
    try {
      const response = await fetch(`http://127.0.0.1:${port}/json/version`, {
        signal: AbortSignal.timeout(Math.max(1, deadline - Date.now())),
      });
      if (response.ok) return await response.json();
    } catch {
      // Chrome can be alive while the debugging service is still starting.
    }
    await delay(200);
  }
  throw new Error("Chrome did not open its debugging port in two minutes");
}
