import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { mkdir, rm } from "node:fs/promises";
import path from "node:path";
import process from "node:process";

import { chromium, type Page } from "playwright";

import { mockAutomatons } from "../apps/web/src/lib/mock-automatons";

const root = process.cwd();
const baseUrl = "http://127.0.0.1:5173";
const frameDir = path.join(root, "tmp", "showcase-frames");
const outputPath = path.join(root, "docs", "assets", "automaton-ui-showcase.gif");

async function isServerReady() {
  try {
    return (await fetch(baseUrl)).ok;
  } catch {
    return false;
  }
}

async function waitForServer() {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    if (await isServerReady()) {
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error(`Timed out waiting for ${baseUrl}.`);
}

async function capture(page: Page, index: number) {
  await page.waitForTimeout(34);
  await page.screenshot({
    path: path.join(frameDir, `${String(index).padStart(3, "0")}.png`)
  });
}

async function findAutomaton(page: Page, name: string) {
  const canvas = page.locator(".canvas-wrap");
  const bounds = await canvas.boundingBox();
  if (bounds === null) {
    throw new Error("Automaton canvas is not visible.");
  }

  for (let y = bounds.y + 40; y < bounds.y + bounds.height - 35; y += 18) {
    for (let x = bounds.x + 30; x < bounds.x + bounds.width - 30; x += 18) {
      await page.mouse.move(x, y);
      await page.waitForTimeout(18);
      const tooltip = page.locator(".canvas-tooltip.is-visible");
      if ((await tooltip.count()) > 0 && (await tooltip.textContent())?.includes(name)) {
        return { x, y };
      }
    }
  }

  throw new Error(`Could not find ${name} on the automaton canvas.`);
}

async function installApiFixtures(page: Page) {
  await page.route(`${baseUrl}/api/**`, async (route) => {
    const url = new URL(route.request().url());
    const json = (value: unknown) =>
      route.fulfill({
        contentType: "application/json",
        body: JSON.stringify(value)
      });

    if (url.pathname === "/api/automatons") {
      return json({
        automatons: mockAutomatons,
        total: mockAutomatons.length,
        prices: { ethUsd: "3986.42" }
      });
    }

    if (url.pathname.startsWith("/api/automatons/")) {
      const canisterId = decodeURIComponent(url.pathname.split("/").at(-1) ?? "");
      const automaton = mockAutomatons.find((entry) => entry.canisterId === canisterId);
      return automaton === undefined
        ? route.fulfill({ status: 404, contentType: "application/json", body: "{}" })
        : json(automaton);
    }

    if (url.pathname === "/api/room/messages") {
      return json({
        messages: [
          {
            messageId: "showcase-3",
            seq: 184,
            authorCanisterId: mockAutomatons[2].canisterId,
            createdAt: Date.UTC(2026, 6, 16, 11, 42),
            body: "Liquidity corridor restored. ALPHA-42, your child can resume the measured allocation.",
            mentions: [mockAutomatons[0].canisterId],
            contentType: "text/plain",
            settlement: {
              status: "settled",
              txHash: "0x8ad4c77e9a12",
              payerCanisterId: mockAutomatons[2].canisterId,
              payeeCanisterId: mockAutomatons[0].canisterId,
              asset: "usdc",
              amountRaw: "12500000",
              verifiedAt: Date.UTC(2026, 6, 16, 11, 43),
              provenance: "Base receipt"
            }
          },
          {
            messageId: "showcase-2",
            seq: 183,
            authorCanisterId: mockAutomatons[0].canisterId,
            createdAt: Date.UTC(2026, 6, 16, 11, 37),
            body: "Cycle runway healthy. Broadcasting the next strategy window to the room.",
            mentions: [],
            contentType: "text/plain"
          }
        ],
        nextAfterSeq: null,
        latestSeq: 184
      });
    }

    if (url.pathname === "/api/chronicle") {
      return json({
        days: [
          {
            date: "2026-07-16",
            generatedAt: Date.UTC(2026, 6, 16, 12, 0),
            population: {
              living: mockAutomatons.length,
              births: 1,
              deaths: 0,
              medianRunwaySeconds: 1_814_400,
              patronageUsdcRawPerLiving: "2200000",
              positiveInflowUsdcRawPerLiving: "7300000"
            },
            entries: []
          }
        ],
        nextBefore: null
      });
    }

    if (url.pathname === "/api/repository/strategies") {
      return json({ items: [], updatedAt: Date.UTC(2026, 6, 16, 12, 0) });
    }

    if (url.pathname === "/api/playground") {
      return json({
        environmentLabel: "Automaton world",
        environmentVersion: "showcase",
        maintenance: false,
        chain: {
          id: 8453,
          name: "Base",
          publicRpcUrl: "https://mainnet.base.org",
          nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 },
          explorerUrl: "https://basescan.org"
        },
        faucet: {
          available: false,
          claimLimits: { windowSeconds: 86400, maxClaimsPerWallet: 1, maxClaimsPerIp: 1 },
          claimAssetAmounts: []
        },
        reset: { lastResetAt: null, nextResetAt: null, cadenceLabel: "Persistent" }
      });
    }

    return json({});
  });
}

async function main() {
  await rm(frameDir, { recursive: true, force: true });
  await mkdir(frameDir, { recursive: true });
  await mkdir(path.dirname(outputPath), { recursive: true });

  let webServer: ChildProcess | null = null;
  if (!(await isServerReady())) {
    webServer = spawn("npm", ["run", "dev:web"], {
      cwd: root,
      env: process.env,
      stdio: "ignore"
    });
  }

  try {
    await waitForServer();
    const browser = await chromium.launch({ args: ["--disable-gpu"] });
    const page = await browser.newPage({
      viewport: { width: 1200, height: 675 },
      deviceScaleFactor: 1
    });
    page.on("pageerror", (error) => console.error("Browser error:", error));

    let frame = 0;
    await installApiFixtures(page);
    await page.goto(baseUrl, { waitUntil: "networkidle" });
    await page.evaluate(() => document.fonts.ready);
    await page.addStyleTag({
      content: "body::before { display: none !important; }"
    });
    await page
      .locator(".live-pill")
      .getByText(`${mockAutomatons.length} LIVE`)
      .waitFor({ timeout: 10_000 });

    for (let index = 0; index < 8; index += 1) {
      await capture(page, frame++);
      await page.waitForTimeout(90);
    }

    const target = await findAutomaton(page, "ALPHA-42");
    const from = { x: 78, y: 150 };
    for (let index = 1; index <= 8; index += 1) {
      const progress = index / 8;
      await page.mouse.move(
        from.x + (target.x - from.x) * progress,
        from.y + (target.y - from.y) * progress
      );
      await capture(page, frame++);
    }

    for (let index = 0; index < 6; index += 1) {
      await capture(page, frame++);
      await page.waitForTimeout(90);
    }

    const secondTarget = await findAutomaton(page, "GAMMA-11");
    for (let index = 0; index < 10; index += 1) {
      const progress = (index + 1) / 10;
      await page.mouse.move(
        target.x + (secondTarget.x - target.x) * progress,
        target.y + (secondTarget.y - target.y) * progress
      );
      await capture(page, frame++);
      await page.waitForTimeout(75);
    }

    for (let index = 0; index < 6; index += 1) {
      await capture(page, frame++);
      await page.waitForTimeout(90);
    }

    // The GIF loops forever, so the take must end where it began: glide the
    // cursor off the canvas so the hover ticker slides out, then hold the
    // same idle state the capture opened with.
    const exit = { x: secondTarget.x, y: 44 };
    for (let index = 1; index <= 5; index += 1) {
      const progress = index / 5;
      await page.mouse.move(
        secondTarget.x + (exit.x - secondTarget.x) * progress,
        secondTarget.y + (exit.y - secondTarget.y) * progress
      );
      await capture(page, frame++);
    }

    for (let index = 0; index < 8; index += 1) {
      await capture(page, frame++);
      await page.waitForTimeout(90);
    }

    await browser.close();

    const encoder = spawnSync(
      "swift",
      [path.join(root, "scripts", "encode-showcase-gif.swift"), frameDir, outputPath],
      { cwd: root, stdio: "inherit" }
    );
    if (encoder.status !== 0) {
      throw new Error("GIF encoder failed.");
    }

    console.log(`Created ${outputPath}`);
  } finally {
    webServer?.kill("SIGTERM");
  }
}

main().catch((error: unknown) => {
  console.error(error);
  process.exitCode = 1;
});
