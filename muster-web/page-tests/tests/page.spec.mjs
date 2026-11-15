import { expect, test } from "@playwright/test";

import { face, floor, snapshot, startFake } from "./fake.mjs";

const MARKUP_LINE = "<img src=x onerror=window.__pwned=1>";
const SIZES = [
  { width: 1280, height: 800 },
  { width: 1024, height: 640 },
];
const BANNERS = {
  absent: "The muster service is not running",
  starting: "The muster service is starting",
  "other-version": "Another version of muster's service is running",
  "no-state": "muster has no state yet",
  "newer-schema": "A newer muster wrote this database",
  unreadable: "muster's database could not be read",
};

let fake;
test.beforeEach(async () => {
  fake = await startFake();
});
test.afterEach(async () => {
  await fake.close();
});

/** The face of `name` in room `room`. */
const faceIn = (page, room, name) =>
  page.locator(`section.room[data-room="${room}"] button.person[data-name="${name}"]`);

test("a state message moves the face to the right room", async ({ page }) => {
  fake.script = [snapshot(1, [floor("demo", [face("web"), face("api")])])];
  await page.goto(fake.url);
  await expect(faceIn(page, "running", "web")).toBeVisible();

  fake.send({
    kind: "process",
    seq: 2,
    project: "demo",
    process: face("web", { room: "failed", state: "crashed", ended: { text: "exit 1", at: null } }),
  });

  await expect(faceIn(page, "failed", "web")).toBeVisible();
  await expect(faceIn(page, "running", "web")).toHaveCount(0);
  await expect(faceIn(page, "running", "api")).toBeVisible();
});

test("a stale message is ignored", async ({ page }) => {
  fake.script = [snapshot(1, [floor("demo", [face("web"), face("api")])])];
  await page.goto(fake.url);
  await expect(faceIn(page, "running", "web")).toBeVisible();

  const moved = (name, room, seq) => ({
    kind: "process",
    seq,
    project: "demo",
    process: face(name, { room, state: room === "off" ? "stopped" : "crashed" }),
  });
  fake.send(moved("web", "failed", 5));
  fake.send(moved("web", "off", 4));
  fake.send(moved("api", "failed", 6));

  await expect(faceIn(page, "failed", "api")).toBeVisible();
  await expect(faceIn(page, "failed", "web")).toBeVisible();
  await expect(faceIn(page, "off", "web")).toHaveCount(0);
});

test("a log line with markup renders as text", async ({ page }) => {
  fake.script = [snapshot(1, [floor("demo", [face("web")])])];
  fake.lines = [MARKUP_LINE];
  await page.goto(fake.url);

  await page.locator('button.person[data-name="web"]').click();

  await expect(page.locator("aside.desk pre.log")).toContainText(MARKUP_LINE);
  expect(await page.evaluate(() => window.__pwned)).toBeUndefined();
  await expect(page.locator("aside.desk img")).toHaveCount(0);
});

test("the page reconnects after the stream drops and shows the new snapshot", async ({ page }) => {
  fake.script = [snapshot(1, [floor("demo", [face("web")])])];
  await page.goto(fake.url);
  await expect(faceIn(page, "running", "web")).toBeVisible();
  expect(fake.streams).toBe(1);

  fake.script = [snapshot(2, [floor("demo", [face("worker", { room: "failed", state: "crashed" })])])];
  fake.drop();

  await expect(faceIn(page, "failed", "worker")).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('button.person[data-name="web"]')).toHaveCount(0);
  expect(fake.streams).toBe(2);
});

for (const size of SIZES) {
  test(`the document never scrolls on a crowded floor at ${size.width}x${size.height}`, async ({
    page,
  }) => {
    const rooms = ["running", "failed", "starting", "restarting", "off"];
    const projects = Array.from({ length: 12 }, (_, p) =>
      floor(
        `project-${p}`,
        Array.from({ length: 5 }, (_, i) => {
          const room = rooms[(p + i) % rooms.length];
          return face(`proc-${p}-${i}`, { room, state: room === "off" ? "stopped" : room });
        }),
      ),
    );
    const notes = {
      "project-0": Array.from({ length: 40 }, (_, n) => ({
        key: `note-${n}`,
        body: `A note body number ${n} with enough words to wrap across a line or two of the panel.`,
        author: "claude",
        label: "claude",
        at: Math.floor(Date.now() / 1000) - n * 60,
      })),
    };
    fake.script = [snapshot(1, projects, { here: "project-0", notes })];
    await page.setViewportSize(size);
    await page.goto(fake.url);
    await expect(page.locator("button.person").first()).toBeVisible();

    await page.locator("button#notes-chip").click();
    await expect(page.locator("article.note").first()).toBeVisible();

    const overflow = await page.evaluate(() => {
      const root = document.scrollingElement;
      return {
        vertical: root.scrollHeight - root.clientHeight,
        horizontal: root.scrollWidth - root.clientWidth,
      };
    });
    expect(overflow).toEqual({ vertical: 0, horizontal: 0 });
  });
}

test("the page fetches nothing it does not serve", async ({ page }) => {
  fake.script = [snapshot(1, [floor("demo", [face("web")])])];
  fake.lines = ["hello"];
  const failed = [];
  const refused = [];
  const foreign = [];
  page.on("requestfailed", (request) => failed.push(request.url()));
  page.on("request", (request) => {
    if (!request.url().startsWith(fake.origin) && !request.url().startsWith("data:")) {
      foreign.push(request.url());
    }
  });
  page.on("response", (response) => {
    if (response.status() >= 400) refused.push(response.url());
  });

  await page.goto(fake.url);
  await page.locator('button.person[data-name="web"]').click();
  await expect(page.locator("aside.desk pre.log")).toContainText("hello");
  await page.waitForLoadState("networkidle").catch(() => {});

  expect(failed).toEqual([]);
  expect(refused).toEqual([]);
  expect(foreign).toEqual([]);
  expect(fake.unserved).toEqual([]);
});

for (const [health, title] of Object.entries(BANNERS)) {
  test(`the ${health} health shows its banner`, async ({ page }) => {
    fake.script = [snapshot(1, [], { health, here: null })];
    await page.goto(fake.url);

    await expect(page.locator(`div.banner[data-health="${health}"]`)).toContainText(title);
  });
}
