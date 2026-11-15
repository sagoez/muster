import { expect, test } from "@playwright/test";

import { face, floor, snapshot, startFake } from "./fake.mjs";

const MARKUP_LINE = "<img src=x onerror=window.__pwned=1>";
const SIZES = [
  { width: 1280, height: 800 },
  { width: 1024, height: 640 },
];
const BANNERS = {
  loading: "Reading muster's state",
  absent: "The muster service is not running",
  starting: "The muster service is starting",
  "other-version": "Another version of muster's service is running",
  unresponsive: "The muster service is not answering",
  "no-state": "muster has no state yet",
  "older-schema": "An older muster wrote this database",
  "newer-schema": "A newer muster wrote this database",
  unreadable: "muster's database could not be read",
};
/** Each way a card can lead into its command, with a process that earns it. */
const NEXT_STEPS = [
  {
    case: "paused",
    fields: { room: "off", mark: "paused", state: "paused" },
    next: { verb: "resume", command: "muster -p demo resume web" },
    lead: "Resume it with",
  },
  {
    case: "unsupervised",
    fields: { room: "running", mark: "muted", state: "unsupervised" },
    next: { verb: "stop", command: "muster -p demo stop web" },
    lead: "Stop it with",
  },
  {
    case: "blocked",
    fields: { room: "failed", state: "blocked" },
    next: { verb: "notices", command: "muster -p demo notices" },
    lead: "See why with",
  },
  {
    case: "crashed",
    fields: { room: "failed", state: "crashed" },
    next: { verb: "logs", command: "muster -p demo logs web" },
    lead: "See why with",
  },
  {
    case: "exited",
    fields: { room: "off", state: "exited" },
    next: { verb: "start", command: "muster -p demo start web" },
    lead: "Start it with",
  },
];

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

for (const step of NEXT_STEPS) {
  test(`a ${step.case} process's card suggests ${step.next.verb}`, async ({ page }) => {
    fake.script = [snapshot(1, [floor("demo", [face("web", { ...step.fields, next: step.next })])])];
    await page.goto(fake.url);

    await page.locator('button.person[data-name="web"]').click();

    const next = page.locator("aside.desk p.next");
    await expect(next).toHaveAttribute("data-verb", step.next.verb);
    await expect(next).toHaveText(`${step.lead} ${step.next.command}`);
  });
}

test("a project whose stack did not read says so, and the other floors still show", async ({ page }) => {
  fake.script = [
    snapshot(1, [floor("demo", [face("web")]), floor("broken", [], { unreadable: true })]),
  ];
  await page.goto(fake.url);
  await expect(faceIn(page, "running", "web")).toBeVisible();
  await expect(page.locator('nav.side button[data-project="broken"]')).toContainText("Unreadable");

  await page.locator('nav.side button[data-project="broken"]').click();

  await expect(page.locator(".floor > .empty[data-unreadable]")).toContainText("could not be read");
  await expect(page.locator("div.banner")).toHaveCount(0);
});

test("a project named after an Object key, added after the snapshot, shows its floor", async ({ page }) => {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  fake.script = [snapshot(1, [floor("demo", [face("web")])])];
  await page.goto(fake.url);
  await expect(page.locator('button.person[data-name="web"]')).toBeVisible();

  fake.send({
    kind: "projects",
    seq: 2,
    here: "demo",
    projects: [floor("demo", [face("web")]), floor("constructor", [face("api")])],
  });
  await page.locator('nav.side button[data-project="constructor"]').click();

  await expect(page.locator('button.person[data-name="api"]')).toBeVisible();
  await expect(page.locator("button#notes-chip")).toContainText("Nothing left yet");
  expect(errors).toEqual([]);
});

test("a removed project's notes do not show on a new project of that name", async ({ page }) => {
  const old = { key: "plan", body: "left by the old project", author: "claude", label: "claude", at: 1 };
  fake.script = [snapshot(1, [floor("demo", [face("web")]), floor("shop", [face("api")])], { notes: { shop: [old] } })];
  await page.goto(`${fake.url}#/shop`);
  await expect(page.locator("button#notes-chip")).toContainText(old.body);

  fake.send({ kind: "projects", seq: 2, here: "demo", projects: [floor("demo", [face("web")])] });
  fake.send({
    kind: "projects",
    seq: 3,
    here: "demo",
    projects: [floor("demo", [face("web")]), floor("shop", [face("worker")])],
  });

  await expect(page.locator('button.person[data-name="worker"]')).toBeVisible();
  await expect(page.locator("button#notes-chip")).toContainText("Nothing left yet");
});

test("the floor shown by default stays put as its processes change", async ({ page }) => {
  fake.script = [
    snapshot(1, [floor("quiet", [face("db", { room: "off", state: "exited" })]), floor("busy", [face("web")])], {
      here: null,
    }),
  ];
  await page.goto(fake.url);
  await expect(page.locator('button.person[data-name="web"]')).toBeVisible();

  fake.send({ kind: "process", seq: 2, project: "busy", process: face("web", { room: "off", state: "exited" }) });
  fake.send({ kind: "process", seq: 3, project: "quiet", process: face("db") });

  await expect(page.locator('section.room[data-room="off"] button.person[data-name="web"]')).toBeVisible();
  await expect(page.locator('button.person[data-name="db"]')).toHaveCount(0);
});

test("the default floor is not chosen while the statuses load", async ({ page }) => {
  fake.script = [snapshot(1, [floor("quiet", []), floor("busy", [])], { here: null, health: "loading" })];
  await page.goto(fake.url);
  await expect(page.locator("div.banner")).toContainText(BANNERS.loading);

  fake.send(
    snapshot(2, [floor("quiet", [face("db", { room: "off", state: "exited" })]), floor("busy", [face("web")])], {
      here: null,
    }),
  );

  await expect(page.locator('section.room[data-room="running"] button.person[data-name="web"]')).toBeVisible();
  await expect(page.locator('button.person[data-name="db"]')).toHaveCount(0);
});

test("a service that is down keeps each process's own word and mark, unconfirmed", async ({ page }) => {
  const unconfirmed = (name, fields) => face(name, { ...fields, unconfirmed: true });
  fake.script = [
    snapshot(
      1,
      [
        floor("demo", [
          unconfirmed("web", {}),
          unconfirmed("worker", { room: "failed", state: "crashed" }),
          unconfirmed("cron", { room: "off", mark: "paused", state: "paused" }),
        ]),
      ],
      { health: "absent" },
    ),
  ];
  await page.goto(fake.url);
  await expect(page.locator("div.banner")).toContainText(BANNERS.absent);

  for (const [room, name, word, mark] of [
    ["running", "web", "Running", null],
    ["failed", "worker", "Failed", null],
    ["off", "cron", "Paused", "paused"],
  ]) {
    const person = faceIn(page, room, name);
    await expect(person.locator(".face")).toHaveAttribute("data-unconfirmed", "");
    if (mark) await expect(person.locator(".face")).toHaveAttribute("data-mark", mark);
    else await expect(person.locator(".face")).not.toHaveAttribute("data-mark", /.*/);
    await person.click();
    await expect(page.locator("aside.desk p.now")).toHaveText(word);
    await expect(page.locator("aside.desk p.unconfirmed")).toContainText("Not confirmed");
    await page.keyboard.press("Escape");
  }
});

test("a project whose stack stops reading keeps its processes under the note", async ({ page }) => {
  fake.script = [snapshot(1, [floor("demo", [face("web", { unconfirmed: true })], { unreadable: true })])];
  await page.goto(fake.url);

  await expect(page.locator(".floor > .empty[data-unreadable]")).toContainText("could not be read");
  await expect(faceIn(page, "running", "web").locator(".face")).toHaveAttribute("data-unconfirmed", "");
});
