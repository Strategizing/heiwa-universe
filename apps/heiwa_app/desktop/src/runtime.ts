import { Channel, invoke } from "@tauri-apps/api/core";
import type { OperatorFrame } from "./operator/types";

// Dev-only Deno herd bridge, reached when the Tauri command layer is absent
// (plain `vite dev` in a browser). The Rust path honors HEIWA_HERD_URL; this
// is the equivalent escape hatch for the dev fallback.
const HERD_BRIDGE_URL: string =
  (import.meta.env.VITE_HERD_BRIDGE_URL as string | undefined) ?? "http://127.0.0.1:7480";

export type ApiErrorPayload =
  | { kind: "Offline"; detail: string }
  | { kind: "Http"; detail: { status: number; body: string } }
  | { kind: "Decode"; detail: string }
  | { kind: "InvalidPath"; detail: string }
  | { kind: "AuthNotConfigured" };

export type RuntimeHealth = {
  reachable: boolean;
  snapshot?: RuntimeSnapshotEnvelope | null;
  error?: ApiErrorPayload | null;
};

export type RuntimeSnapshotEnvelope = {
  ok?: boolean;
  data?: {
    /**
     * The runtime block the snapshot actually returns. `runtime_version` at
     * the top level was never a field the runtime sends — the version lives
     * here, which is why every screen showed "unknown".
     */
    runtime?: {
      version?: string;
      status?: string;
      started_at?: string;
      node?: string;
    };
    /** Kept for older snapshots that flattened it. */
    runtime_version?: string;
    started_at?: string;
    status?: string;
    notes?: string[];
    providers?: ProviderSnapshot[];
    resource?: ResourceSnapshot;
    machine?: MachineSnapshot;
    workers?: WorkersSnapshot;
    approvals?: ApprovalsSnapshot;
    [key: string]: unknown;
  };
  [key: string]: unknown;
};

export type ProviderSnapshot = {
  provider_id: string;
  display_name: string;
  status: string;
  auth_kind: string;
  default_model: string | null;
  supported_lanes: string[];
  last_error: string | null;
  last_validated_at: string | null;
};

export type ResourceSnapshot = {
  snapshot?: {
    cpu_count?: number;
    free_memory_bytes?: number;
    load_1m?: number;
    battery_percent?: number | null;
    on_battery?: boolean;
    thermal_pressure?: string;
  };
  admissions?: Record<string, { decision: string }>;
};

export type MachineSnapshot = {
  schema_version?: string;
  device_id?: string | null;
  display_name?: string;
  hostname?: string;
  os?: string;
  arch?: string;
  device_class?: string;
  hardware?: {
    logical_cpu_count?: number;
    memory_total_bytes?: number;
    cpu_model?: string | null;
    hardware_model?: string | null;
  };
  capabilities?: {
    provider_clis?: string[];
    local_model_runtimes?: string[];
    host_surfaces?: string[];
    display_surfaces?: string[];
  };
  runtime?: {
    version?: string;
    channel?: string;
    install_path?: string | null;
  };
  perspective?: {
    locality?: string;
    execution_scope?: string;
    data_scope?: string;
    sync_status?: string;
    transport?: string;
    node_id?: string | null;
    enrolled_peer_count?: number;
    enrolled_peer_ids?: string[];
    mesh_errors?: { code?: string; message?: string }[];
  };
  recognition_error?: {
    code?: string;
    message?: string;
  };
};

export type WorkersSnapshot = {
  total?: number;
  live?: number;
  stale?: number;
  /** Runtime hosts keep Heiwa available; they are not user-dispatched work. */
  runtime_live?: number;
  /** Workers currently executing user or automation tasks. */
  task_live?: number;
};

export type ApprovalsSnapshot = {
  pending?: number;
  decided?: number;
};

export type AgentRow = {
  agent_id?: string;
  provider?: string;
  model?: string;
  status?: string;
  lane?: string;
  last_used?: string;
  [key: string]: unknown;
};

export type HerdPane = {
  workspace: string;
  pane: string;
  agent: string;
  state: string;
  cwd: string;
  message?: string;
};

export type HerdSnapshot = {
  status: "checking" | "online" | "offline" | string;
  source: string;
  panes: HerdPane[];
  error?: string | null;
};

export type HerdPaneRead = {
  ok: boolean;
  pane: string;
  text: string;
  source: string;
  error?: string | null;
};

export type HerdActionResult = {
  ok: boolean;
  message: string;
  source: string;
  error?: string | null;
};

export type HerdCommandSpec = {
  id: string;
  label: string;
  command: string;
  risk: string;
  approval: string;
  description: string;
};

export type SubagentDispatchRequest = {
  task: string;
  provider?: string;
  model?: string;
  lane?: "local" | "cloud" | "auto";
  context?: string;
  approval_policy?: "auto" | "ask" | "deny";
};

export type SubagentDispatchResponse = {
  ok?: boolean;
  data?: {
    agent_id?: string;
    provider?: string;
    model?: string;
    status?: string;
    response?: string;
    trace?: Record<string, unknown>;
    error?: string;
  };
};

export type OllamaModel = {
  name: string;
  size?: number;
  modified?: string;
  parameter_size?: string;
  quantization_level?: string;
};

/** A published release newer than the running one, as the shell offers it. */
export type UpdateOffer = {
  version: string;
  current_version: string;
  notes?: string | null;
};

export async function runtimeHealth(): Promise<RuntimeHealth> {
  return invoke<RuntimeHealth>("runtime_health");
}

/**
 * Whether a newer signed bundle is published. `undefined` covers both "up to
 * date" and "nothing to ask" — a browser-only dev server has no updater, and
 * an unreachable manifest is not something to put in front of the user.
 */
export async function checkForUpdate(): Promise<UpdateOffer | undefined> {
  try {
    return (await invoke<UpdateOffer | null>("update_check")) ?? undefined;
  } catch {
    return undefined;
  }
}

/**
 * Install the offered update and relaunch into it.
 *
 * Errors propagate: the banner shows why, because silently doing nothing
 * would leave the user thinking they had updated.
 */
export async function installUpdate(): Promise<void> {
  return invoke<void>("update_install");
}

export async function apiGet<T>(path: string): Promise<T> {
  return invoke<T>("api_get", { path });
}

export async function apiPost<T>(path: string, body: unknown): Promise<T> {
  return invoke<T>("api_post", { path, body });
}

export async function operatorSubscribe(
  threadId: string,
  after: string | null,
  onFrame: (frame: OperatorFrame) => void,
): Promise<void> {
  const onEvent = new Channel<OperatorFrame>(onFrame);
  return invoke<void>("operator_subscribe", { threadId, after, onEvent });
}

export async function dispatchSubagent(req: SubagentDispatchRequest): Promise<SubagentDispatchResponse> {
  return apiPost<SubagentDispatchResponse>("/api/v1/agents/dispatch", req);
}

export async function listOllamaModels(): Promise<{ models: OllamaModel[] }> {
  return apiGet<{ models: OllamaModel[] }>("/api/v1/providers/ollama/models");
}

export async function herdPanes(): Promise<HerdSnapshot> {
  try {
    return await invoke<HerdSnapshot>("herd_panes");
  } catch {
    try {
      const resp = await fetch(`${HERD_BRIDGE_URL}/api/herd`, { cache: "no-store" });
      if (!resp.ok) throw new Error(`herd ${resp.status}`);
      const panes = await resp.json() as HerdPane[];
      return { status: "online", source: "deno-bridge-dev", panes, error: null };
    } catch (error) {
      return {
        status: "offline",
        source: "none",
        panes: [],
        error: error instanceof Error ? error.message : String(error),
      };
    }
  }
}

export async function herdCommandCatalog(): Promise<HerdCommandSpec[]> {
  try {
    return await invoke<HerdCommandSpec[]>("herd_command_catalog");
  } catch {
    return [
      {
        id: "git.status",
        label: "Git status",
        command: "git status --short --branch",
        risk: "host_safe_readonly",
        approval: "auto",
        description: "Show the current branch and dirty worktree without mutating files.",
      },
      {
        id: "git.diff.stat",
        label: "Git diff stat",
        command: "git diff --stat",
        risk: "host_safe_readonly",
        approval: "auto",
        description: "Summarize unstaged file changes without printing full diff contents.",
      },
      {
        id: "monitor.ops",
        label: "Monitor ops",
        command: "heiwa app api get /api/v1/monitor --json",
        risk: "host_safe_readonly",
        approval: "auto",
        description: "Read combined user and machine ops state from the local Heiwa.app runtime.",
      },
      {
        id: "monitor.machine",
        label: "Monitor machine",
        command: "heiwa app api get /api/v1/resource --json",
        risk: "host_safe_readonly",
        approval: "auto",
        description: "Read CPU, memory, thermal, and admission state.",
      },
      {
        id: "monitor.inbox",
        label: "Monitor inbox",
        command: "heiwa app api get /api/v1/inbox --json",
        risk: "host_safe_readonly",
        approval: "auto",
        description: "Read the local intake inbox for receipts and operator-facing items.",
      },
    ];
  }
}

export async function readHerdPane(pane: string): Promise<HerdPaneRead> {
  try {
    return await invoke<HerdPaneRead>("herd_pane_read", { pane });
  } catch {
    try {
      const resp = await fetch(`${HERD_BRIDGE_URL}/api/pane/${encodeURIComponent(pane)}?format=text`, {
        cache: "no-store",
      });
      const text = await resp.text();
      if (!resp.ok) throw new Error(text || `pane read ${resp.status}`);
      return { ok: true, pane, text, source: "deno-bridge-dev", error: null };
    } catch (error) {
      return {
        ok: false,
        pane,
        text: "",
        source: "none",
        error: error instanceof Error ? error.message : String(error),
      };
    }
  }
}

export async function sendHerdPane(pane: string, text: string): Promise<HerdActionResult> {
  try {
    return await invoke<HerdActionResult>("herd_pane_send", { pane, text });
  } catch {
    return postHerdPaneAction(pane, "send", text);
  }
}

export async function runHerdPane(pane: string, command: string): Promise<HerdActionResult> {
  try {
    return await invoke<HerdActionResult>("herd_pane_run", { pane, command });
  } catch {
    return postHerdPaneAction(pane, "run", command);
  }
}

export async function focusHerdPane(pane: string): Promise<HerdActionResult> {
  try {
    return await invoke<HerdActionResult>("herd_pane_focus", { pane });
  } catch {
    return postHerdPaneAction(pane, "focus", "");
  }
}

export async function splitHerdPane(
  pane: string,
  direction: "right" | "down" = "right",
  cwd?: string,
): Promise<HerdActionResult> {
  try {
    return await invoke<HerdActionResult>("herd_pane_split", { pane, direction, cwd });
  } catch {
    return postHerdPaneAction(pane, "split", JSON.stringify({ direction, cwd }));
  }
}

async function postHerdPaneAction(
  pane: string,
  action: "send" | "run" | "focus" | "split",
  body: string,
): Promise<HerdActionResult> {
  try {
    const resp = await fetch(
      `${HERD_BRIDGE_URL}/api/pane/${encodeURIComponent(pane)}/${action}`,
      { method: "POST", body },
    );
    const text = await resp.text();
    if (!resp.ok) throw new Error(text || `${action} ${resp.status}`);
    return { ok: true, message: text || `${action} ok`, source: "deno-bridge-dev", error: null };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return { ok: false, message, source: "none", error: message };
  }
}

export function runtimeVersion(health: RuntimeHealth | null): string {
  const data = health?.snapshot?.data;
  return data?.runtime?.version ?? data?.runtime_version ?? "unknown";
}

export function runtimeStatus(health: RuntimeHealth | null): string {
  if (!health) return "checking";
  if (!health.reachable) return "offline";
  const data = health.snapshot?.data;
  return data?.runtime?.status ?? data?.status ?? "ok";
}

export function providersFromSnapshot(health: RuntimeHealth | null): ProviderSnapshot[] {
  return health?.snapshot?.data?.providers ?? [];
}

export function resourceFromSnapshot(health: RuntimeHealth | null): ResourceSnapshot | null {
  return health?.snapshot?.data?.resource ?? null;
}

export function workersFromSnapshot(health: RuntimeHealth | null): WorkersSnapshot | null {
  return health?.snapshot?.data?.workers ?? null;
}

export function approvalsFromSnapshot(health: RuntimeHealth | null): ApprovalsSnapshot | null {
  return health?.snapshot?.data?.approvals ?? null;
}
