// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { App } from "./app";
import { localIsoDate } from "./lib/format";
import { createAppState, type AppState } from "./state/app";
import { SURFACES } from "./surfaces/registry";
import type { OperatorFrame } from "./operator/types";

afterEach(cleanup);

const EMPTY_HISTORY = {
  ok: true,
  data: { events: [], next_cursor: null, skipped_lines: 0 },
};

type Harness = {
  state: AppState;
  post: ReturnType<typeof vi.fn>;
  runtimePost: ReturnType<typeof vi.fn>;
  emit: (frame: OperatorFrame) => void;
};

/**
 * Builds app state with every runtime dependency faked, so the shell is
 * exercised without a live runtime. `schedule` runs inline: the production
 * scheduler defers publishes to an animation frame, which a test would have
 * to wait on.
 */
function harness(
  overrides: {
    subscribeNever?: boolean;
    get?: (path: string) => Promise<unknown>;
    machineOs?: string;
    machineName?: string;
    machineRecognitionError?: { code: string; message: string };
    machineSyncStatus?: string;
    emptyHerd?: boolean;
  } = {},
): Harness {
  const post = vi.fn().mockResolvedValue({
    ok: true,
    data: {
      thread_id: "default",
      turn_id: "turn-1",
      cursor: "1",
      duplicate: false,
      stream_url: "/stream",
    },
  });
  const runtimePost = vi.fn().mockResolvedValue({ ok: true, data: {} });
  let emit: (frame: OperatorFrame) => void = () => {};

  const state = createAppState({
    operator: {
      get: vi.fn().mockResolvedValue(EMPTY_HISTORY),
      post,
      subscribe: (_thread, _after, onFrame) => {
        emit = onFrame;
        return overrides.subscribeNever ? new Promise<void>(() => {}) : Promise.resolve();
      },
      randomUUID: () => "req-1",
      schedule: (task) => task(),
    },
    runtime: {
      get: overrides.get
        ? (vi.fn(overrides.get) as never)
        : vi.fn().mockResolvedValue({ data: { items: [], holds: [], events: [] } }),
      health: vi.fn().mockResolvedValue({
        reachable: true,
        error: null,
        snapshot: {
          ok: true,
          data: {
            // The shape the runtime actually returns. The old fixture put
            // the version at the top level, where nothing sends it, so the
            // reader could look in the wrong place and still pass.
            runtime: { version: "0.1.0-test", status: "ok" },
            machine: {
              schema_version: "heiwa_machine_v1",
              device_id: "mac-test-device",
              display_name: overrides.machineName ?? "dmac.local",
              hostname: overrides.machineName ?? "dmac.local",
              os: overrides.machineOs ?? "macos",
              arch: "aarch64",
              device_class: "full_node",
              hardware: {
                logical_cpu_count: 12,
                memory_total_bytes: 25_769_803_776,
                cpu_model: "Apple M4 Pro",
                hardware_model: "Mac16,8",
              },
              perspective: {
                locality: "local",
                execution_scope: "this_device",
                data_scope: "shared_user",
                sync_status: overrides.machineSyncStatus ?? "local_only",
              },
              recognition_error: overrides.machineRecognitionError,
            },
            resource: {
              snapshot: {
                cpu_count: 12,
                free_memory_bytes: 12_884_901_888,
                load_1m: 1.2,
                battery_percent: 100,
                on_battery: false,
                thermal_pressure: "nominal",
              },
            },
            workers: {
              total: 1,
              live: 1,
              stale: 0,
              runtime_live: 1,
              task_live: 0,
            },
            providers: [
              {
                provider_id: "ollama",
                display_name: "Ollama",
                status: "connected",
                auth_kind: "local_runtime",
                default_model: null,
                supported_lanes: [],
                last_error: null,
                last_validated_at: null,
              },
            ],
          },
        },
      }),
      post: runtimePost,
    },
    herd: {
      snapshot: vi.fn().mockResolvedValue({
        status: "online",
        source: "test",
        panes: overrides.emptyHerd
          ? []
          : [
              {
                workspace: "heiwa",
                pane: "heiwa:ops",
                agent: "multiplexer",
                state: "running",
                cwd: "/work",
              },
            ],
        error: null,
      }),
      catalog: vi.fn().mockResolvedValue([]),
      read: vi.fn().mockResolvedValue({
        ok: true,
        pane: "heiwa:ops",
        text: "pane output",
        source: "test",
        error: null,
      }),
    },
  });

  return { state, post, runtimePost, emit: (frame) => emit(frame) };
}

/**
 * A distinctive string each surface must put on screen when mounted.
 *
 * These must not collide with rail content: the rail renders every surface's
 * `preview().title` regardless of which surface is active, so keying Mail on
 * "Mail" passed even with the component gutted. Each marker below is body
 * copy only that surface renders.
 */
const SURFACE_MARKERS: Record<string, string | RegExp> = {
  home: "Heiwa Ops",
  ai: "No messages yet.",
  windows: "Terminal panes",
  calendar: "Upcoming",
  approvals: "Pending decisions",
  // Mail now renders the local snapshot rather than an L3 placeholder.
  mail: /metadata only, read\s+from this machine/,
  finance: /Read model arrives with the L3 connector plane/,
  social: /Ingress arrives with the L3 connector plane/,
  workers: "Operator turns",
  browser: "Go",
  files: "Workspace tree",
};

describe("shell", () => {
  it("registers the primary surfaces including in-app approvals", () => {
    expect(SURFACES.map((surface) => surface.id)).toEqual([
      "home",
      "ai",
      "windows",
      "calendar",
      "approvals",
      "mail",
      "finance",
      "social",
      "workers",
      "browser",
      "files",
    ]);
  });

  it("decides a pending dispatch approval through the runtime service", async () => {
    const { state, runtimePost } = harness({
      get: async (path) => {
        if (path === "/api/v1/approvals/summary") {
          return {
            data: {
              pending_count: 1,
              pending: [
                {
                  id: "req_calendar_1",
                  action: "calendar-event-create",
                  target: "calendar:hold-1",
                  risk: "T2",
                  requested_at: "2026-08-21T00:00:00Z",
                },
              ],
              requests_dir: "/state/dispatch/requests",
              decisions_dir: "/state/dispatch/approvals/decisions",
            },
          };
        }
        return { data: { items: [], holds: [], events: [] } };
      },
    });
    state.navigate("approvals");
    render(() => <App state={state} />);

    await Promise.resolve();
    await Promise.resolve();
    screen.getByRole("button", { name: /approve req_calendar_1/i }).click();
    await Promise.resolve();

    expect(runtimePost).toHaveBeenCalledWith(
      "/api/v1/approvals/req_calendar_1/approve",
      {},
    );
  });

  it("keeps Apple Calendar disconnected until the user connects it", async () => {
    const { state, runtimePost } = harness({
      get: async (path) => {
        if (path === "/api/v1/calendar/resources") {
          return {
            data: {
              source: "apple_calendar",
              status: "disconnected",
              calendars: [],
              detail: "Detected on this Mac, but not connected to this Heiwa profile.",
              next_action: "heiwa connect apple-calendar --authorize",
            },
          };
        }
        return { data: { items: [], holds: [], events: [] } };
      },
    });
    state.navigate("calendar");
    render(() => <App state={state} />);

    await Promise.resolve();
    await Promise.resolve();
    screen.getByRole("button", { name: /connect apple calendar/i }).click();
    await Promise.resolve();

    expect(runtimePost).toHaveBeenCalledWith(
      "/api/v1/connectors/apple_calendar/connect",
      {},
    );
  });

  it("stages an Apple event for approval from the native Calendar surface", async () => {
    const { state, runtimePost } = harness({
      get: async (path) => {
        if (path === "/api/v1/calendar/resources") {
          return {
            data: {
              source: "apple_calendar",
              status: "ready",
              calendars: [{ name: "Calendar", writable: true }],
              detail: "Connected to this Heiwa profile on this device.",
            },
          };
        }
        return { data: { items: [], holds: [], events: [] } };
      },
    });
    state.navigate("calendar");
    render(() => <App state={state} />);
    await Promise.resolve();
    await Promise.resolve();

    fireEvent.input(screen.getByLabelText("Event title"), {
      target: { value: "Call mom" },
    });
    fireEvent.input(screen.getByLabelText("Event date"), {
      target: { value: "2026-08-22" },
    });
    fireEvent.input(screen.getByLabelText("Event start"), {
      target: { value: "15:00" },
    });
    fireEvent.input(screen.getByLabelText("Event end"), {
      target: { value: "15:30" },
    });
    screen.getByRole("button", { name: /stage apple event/i }).click();
    await Promise.resolve();

    expect(runtimePost).toHaveBeenCalledWith("/api/v1/calendar/holds", {
      title: "Call mom",
      date: "2026-08-22",
      start: "15:00",
      end: "15:30",
      kind: "focus",
      promotion: { connector: "apple_calendar", calendar: "Calendar" },
    });
  });

  it("does not present the live runtime host as a task worker", async () => {
    const { state } = harness();
    state.navigate("workers");
    render(() => <App state={state} />);
    await Promise.resolve();

    expect(screen.getByText("0 task workers")).toBeTruthy();
  });

  it("does not invent panes or active agents for a fresh profile", async () => {
    const { state } = harness({ emptyHerd: true });
    render(() => <App state={state} />);
    await Promise.resolve();
    await Promise.resolve();

    expect(screen.getAllByText(/0 live panes/).length).toBeGreaterThan(0);
    expect(screen.queryByText(/6 sub-app agents/)).toBeNull();
    expect(screen.queryByText("planned")).toBeNull();

    state.navigate("windows");
    expect(screen.queryByText("Sub-app agents")).toBeNull();
  });

  it("renders one rail button per surface", () => {
    const { state } = harness();
    render(() => <App state={state} />);
    for (const surface of SURFACES) {
      expect(screen.getByLabelText(surface.label)).toBeTruthy();
    }
  });

  it.each(SURFACES.map((surface) => surface.id))("mounts the %s surface", (id) => {
    const { state } = harness();
    state.navigate(id);
    const { container } = render(() => <App state={state} />);
    // Scope to the main area: the rail carries every surface's preview title,
    // so an unscoped query can pass on a surface that rendered nothing.
    const main = container.querySelector(".main-area");
    expect(main).toBeTruthy();
    const marker = SURFACE_MARKERS[id];
    const matcher =
      typeof marker === "string"
        ? new RegExp(marker.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))
        : marker;
    expect(matcher.test(main!.textContent ?? "")).toBe(true);
  });

  it("shows the active surface caption in the composer", () => {
    const { state } = harness();
    state.navigate("windows");
    render(() => <App state={state} />);
    expect(screen.getByText("windows")).toBeTruthy();
  });

  it("renders runtime health in the composer hint", async () => {
    const { state } = harness();
    render(() => <App state={state} />);
    await state.runtime.loadHealth();
    expect(screen.getByText(/0\.1\.0-test · 1 providers/)).toBeTruthy();
  });
});

describe("operator seam", () => {
  it("submits a turn through the untouched OperatorClient and lands on AI", async () => {
    const { state, post } = harness();
    render(() => <App state={state} />);
    await state.operator.start("default");
    expect(state.operator.ready()).toBe(true);

    const input = screen.getByLabelText("Message Heiwa") as HTMLTextAreaElement;
    input.value = "hello heiwa";
    screen.getByLabelText("Send").click();
    await Promise.resolve();

    expect(post).toHaveBeenCalledWith("/api/v1/operator/threads/default/turns", {
      client_request_id: "req-1",
      prompt: "hello heiwa",
      route_policy: { mode: "auto" },
    });
    expect(state.view()).toBe("ai");
  });

  it("renders durable events and streaming deltas from the store projection", async () => {
    const { state, emit } = harness({ subscribeNever: true });
    state.navigate("ai");
    render(() => <App state={state} />);
    await state.operator.start("default");

    emit({
      type: "event",
      cursor: "1",
      event: {
        schema_version: 1,
        event_id: "evt-1",
        thread_id: "default",
        turn_id: "turn-1",
        run_id: null,
        call_id: null,
        event_type: "user_message",
        occurred_at: "2026-08-14T00:00:00Z",
        actor: { kind: "user", id: "local" },
        risk_class: "none",
        sensitivity: "normal",
        parent_event_id: null,
        correlation_id: null,
        source_refs: [],
        evidence_refs: [],
        payload: { text: "what changed today?" },
      },
    });
    expect(screen.getByText("what changed today?")).toBeTruthy();

    emit({
      type: "assistant_delta",
      thread_id: "default",
      turn_id: "turn-1",
      text: "three receipts",
    });
    expect(screen.getByText("three receipts")).toBeTruthy();
  });

  it("disables the composer until the stream is ready", () => {
    const { state } = harness();
    render(() => <App state={state} />);
    expect((screen.getByLabelText("Send") as HTMLButtonElement).disabled).toBe(true);
  });

  it("covers the shell with first run until onboarding is complete", () => {
    // Onboarding gates the whole application, so it is an overlay rather
    // than an eleventh surface — there is nothing useful to navigate to
    // before a provider exists.
    const { state } = harness();
    render(() => (
      <App
        state={state}
        onboarding={{
          complete: false,
          display_name: null,
          gaps: [
            {
              step: "provider",
              detail: "no provider account is connected",
              remedy: "add a key with `heiwa auth add-key <provider> <key>`",
            },
          ],
        }}
      />
    ));

    expect(screen.getByText("Set up Heiwa")).toBeTruthy();
    expect(screen.getByText("no provider account is connected")).toBeTruthy();
  });

  it("shows every gap with the action that closes it", () => {
    const { state } = harness();
    render(() => (
      <App
        state={state}
        onboarding={{
          complete: false,
          display_name: null,
          gaps: [
            { step: "identity", detail: "no local identity yet", remedy: "run heiwa setup" },
            { step: "provider", detail: "no provider connected", remedy: "add a key" },
          ],
        }}
      />
    ));

    for (const text of ["no local identity yet", "run heiwa setup", "no provider connected", "add a key"]) {
      expect(screen.getByText(text)).toBeTruthy();
    }
  });

  it("gets out of the way once onboarding is complete", () => {
    const { state } = harness();
    render(() => (
      <App state={state} onboarding={{ complete: true, display_name: "Ada", gaps: [] }} />
    ));

    expect(screen.queryByText("Set up Heiwa")).toBeNull();
    expect(screen.getByLabelText("Send")).toBeTruthy();
  });

  it("renders the shell when onboarding state has not arrived yet", () => {
    // The projection is fetched asynchronously. Blocking the shell on it
    // would make a slow provider probe look like a broken application.
    const { state } = harness();
    render(() => <App state={state} />);

    expect(screen.queryByText("Set up Heiwa")).toBeNull();
    expect(screen.getByLabelText("Send")).toBeTruthy();
  });

  it("shows the messages the local mail snapshot actually holds", () => {
    // The pipeline existed and the surface ignored it: `heiwa mail scan`
    // writes a metadata snapshot from the user's own Mail.app, the runtime
    // serves it at /api/v1/mail/summary, and the surface was rendering a
    // "reads land on L3" placeholder while real data sat one call away.
    const { state } = harness({
      get: async (path: string) =>
        path === "/api/v1/mail/summary"
          ? {
              data: {
                priority: [
                  { sender: "ada@example.com", subject: "Re: launch", unread: true, account: "Work" },
                  { sender: "grace@example.com", subject: "Invoice", unread: false, account: "Work" },
                ],
              },
            }
          : { data: {} },
    });

    render(() => <App state={state} />);
    state.navigate("mail");

    return Promise.resolve().then(() => {
      expect(screen.getByText("Re: launch")).toBeTruthy();
      expect(screen.getByText("ada@example.com")).toBeTruthy();
    });
  });

  it("says the snapshot is empty rather than pretending mail is unsupported", () => {
    // An empty snapshot is a state with an action — run a scan — not the
    // same thing as the feature not existing.
    const { state } = harness({ get: async () => ({ data: { priority: [] } }) });

    render(() => <App state={state} />);
    state.navigate("mail");

    return Promise.resolve().then(() => {
      expect(screen.getByText(/heiwa mail scan/)).toBeTruthy();
    });
  });

  it("opens on a briefing of what today actually holds", () => {
    // The point of having the calendar and the mail locally is that the app
    // can answer "what do I need to know right now" the moment it opens,
    // without the user going and looking in two places.
    // Local, matching what a machine's calendar hands back.
    const today = localIsoDate();
    const { state } = harness({
      get: async (path: string) => {
        if (path === "/api/v1/calendar/summary") {
          return {
            data: {
              events: [
                { id: "e1", title: "Standup", date: today, start: "09:30" },
                { id: "e2", title: "Design review", date: today, start: "14:00" },
                { id: "e3", title: "Next week thing", date: "2099-01-01", start: "10:00" },
              ],
            },
          };
        }
        if (path === "/api/v1/mail/summary") {
          return {
            data: {
              priority: [
                { sender: "ada@example.com", subject: "Re: launch", unread: true },
                { sender: "grace@example.com", subject: "Invoice", unread: false },
              ],
            },
          };
        }
        return { data: {} };
      },
    });

    render(() => <App state={state} />);

    return Promise.resolve()
      .then(() => Promise.resolve())
      .then(() => {
        const briefing = document.querySelector(".today-briefing");
        expect(briefing).toBeTruthy();
        const text = briefing!.textContent ?? "";
        // Today's events only — a briefing that includes next week is a list,
        // not a briefing.
        expect(text).toContain("Standup");
        expect(text).not.toContain("Next week thing");
        expect(text).toContain("1 unread");
      });
  });

  it("opens from this Mac perspective without pretending peer sync exists", () => {
    const { state } = harness();

    render(() => <App state={state} />);

    return Promise.resolve()
      .then(() => Promise.resolve())
      .then(() => {
        const perspective = document.querySelector(".machine-perspective");
        expect(perspective).toBeTruthy();
        const text = perspective!.textContent ?? "";
        expect(text).toContain("This Mac");
        expect(text).toContain("dmac.local");
        expect(text).toContain("Apple M4 Pro");
        expect(text).toContain("12 cores");
        expect(text).toContain("Shared data");
        expect(text).toContain("sync local only");
      });
  });

  it("does not claim peer sync when the mesh state could not be read", () => {
    const { state } = harness({ machineSyncStatus: "unknown" });

    render(() => <App state={state} />);

    return Promise.resolve()
      .then(() => Promise.resolve())
      .then(() => {
        const text = document.querySelector(".machine-perspective")?.textContent ?? "";
        expect(text).not.toContain("peer enrolled");
        expect(text).not.toContain("sync local only");
        expect(text).toContain("sync state unavailable");
      });
  });

  it("renders the same shared-data client from a Windows-local perspective", () => {
    const { state } = harness({ machineOs: "windows", machineName: "devon-windows" });

    render(() => <App state={state} />);

    return Promise.resolve()
      .then(() => Promise.resolve())
      .then(() => {
        const text = document.querySelector(".machine-perspective")?.textContent ?? "";
        expect(text).toContain("This Windows PC");
        expect(text).toContain("devon-windows");
        expect(text).toContain("Shared data");
        expect(text).toContain("sync local only");
      });
  });

  it("explains an incompatible machine manifest without exposing its contents", () => {
    const { state } = harness({
      machineRecognitionError: {
        code: "unsupported_schema",
        message: "Machine identity was written by a newer or incompatible Heiwa build.",
      },
    });

    render(() => <App state={state} />);

    return Promise.resolve()
      .then(() => Promise.resolve())
      .then(() => {
        const perspective = document.querySelector(".machine-perspective");
        expect(perspective?.textContent).toContain("Device recognition needs attention");
        expect(perspective?.textContent).toContain(
          "Machine identity was written by a newer or incompatible Heiwa build.",
        );
      });
  });

  it("says the day is clear rather than showing an empty briefing", () => {
    const { state } = harness({ get: async () => ({ data: { events: [], priority: [] } }) });

    render(() => <App state={state} />);

    return Promise.resolve()
      .then(() => Promise.resolve())
      .then(() => {
        const briefing = document.querySelector(".today-briefing");
        expect(briefing?.textContent ?? "").toMatch(/nothing scheduled/i);
      });
  });

  it("offers a published update and installs it on the user's word", async () => {
    // Registering the updater plugin only makes a release fetchable. This is
    // the reachable path: without a rendered offer wired to the install
    // command, a signed release sits on GitHub and every shell stays stale.
    const { state } = harness();
    const onInstallUpdate = vi.fn().mockResolvedValue(undefined);

    render(() => (
      <App
        state={state}
        update={{ version: "0.2.0", current_version: "0.1.0" }}
        onInstallUpdate={onInstallUpdate}
      />
    ));

    const banner = document.querySelector(".update-banner");
    expect(banner?.textContent ?? "").toContain("0.2.0");

    const action = screen.getByRole("button", { name: /install and relaunch/i });
    action.click();
    await Promise.resolve();

    expect(onInstallUpdate).toHaveBeenCalledTimes(1);
  });

  it("stays quiet when no update is published", () => {
    const { state } = harness();

    render(() => <App state={state} />);

    expect(document.querySelector(".update-banner")).toBeNull();
  });

  it("keeps the offer on screen with the reason when installing fails", async () => {
    // A banner that disappears on failure leaves the user believing they
    // updated when they did not.
    const { state } = harness();
    const onInstallUpdate = vi.fn().mockRejectedValue(new Error("signature rejected"));

    render(() => (
      <App
        state={state}
        update={{ version: "0.2.0", current_version: "0.1.0" }}
        onInstallUpdate={onInstallUpdate}
      />
    ));

    screen.getByRole("button", { name: /install and relaunch/i }).click();
    await Promise.resolve();
    await Promise.resolve();

    const banner = document.querySelector(".update-banner");
    expect(banner?.textContent ?? "").toContain("signature rejected");
  });
});
