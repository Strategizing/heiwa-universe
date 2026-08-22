import { Show } from "solid-js";
import { useApp } from "../../state/app";

function platformLabel(os: string | undefined): string {
  if (os === "macos") return "This Mac";
  if (os === "windows") return "This Windows PC";
  if (os === "linux") return "This Linux machine";
  return "This device";
}

/**
 * Exhaustive, because the runtime can also answer "unknown". A ternary with a
 * fallback would render an unreadable mesh state as "peer enrolled" — the
 * opposite of the truth.
 */
function syncLabel(status: string | undefined): string {
  if (status === "local_only") return "sync local only";
  if (status === "peer_enrolled") return "peer enrolled";
  return "sync state unavailable";
}

function memoryLabel(bytes: number | undefined): string | null {
  if (!bytes) return null;
  return `${(bytes / 1024 ** 3).toFixed(0)} GB memory`;
}

/**
 * Local lens over shared Heiwa state. Identity/capability truth comes from
 * this runtime; Work and evidence can later replicate without pretending the
 * peer transport exists before it does.
 */
export function MachinePerspective() {
  const app = useApp();
  const machine = () => app.runtime.health()?.snapshot?.data?.machine;
  const resource = () => app.runtime.health()?.snapshot?.data?.resource?.snapshot;

  return (
    <section class="panel machine-perspective" aria-label="Local machine perspective">
      <Show
        when={machine()}
        fallback={
          <div class="machine-perspective-loading">
            <span class="quiet">Local perspective</span>
            <strong>Recognizing this device…</strong>
          </div>
        }
      >
        {(current) => {
          const cores = () =>
            current().hardware?.logical_cpu_count ?? resource()?.cpu_count ?? 0;
          const memory = () => memoryLabel(current().hardware?.memory_total_bytes);
          const battery = () => resource()?.battery_percent;
          return (
            <>
              <header class="machine-perspective-header">
                <div>
                  <span class="quiet">Local perspective</span>
                  <h2>{platformLabel(current().os)}</h2>
                  <p>{current().display_name ?? current().hostname ?? "Unnamed device"}</p>
                </div>
                <span class="state-chip local">local</span>
              </header>

              <div class="machine-facts">
                <strong>{current().hardware?.cpu_model ?? current().hardware?.hardware_model ?? current().arch ?? "Unknown hardware"}</strong>
                <span>{cores()} cores</span>
                <Show when={memory()}>{(label) => <span>{label()}</span>}</Show>
                <Show when={battery() !== null && battery() !== undefined}>
                  <span>{battery()}% battery</span>
                </Show>
              </div>

              <Show when={current().recognition_error?.message}>
                {(message) => (
                  <div class="machine-recognition-error" role="alert">
                    <strong>Device recognition needs attention</strong>
                    <span>{message()}</span>
                  </div>
                )}
              </Show>

              <div class="machine-scope">
                <div>
                  <strong>Shared data</strong>
                  <span>Work and evidence use one user scope.</span>
                </div>
                <span class="state-chip planned">{syncLabel(current().perspective?.sync_status)}</span>
              </div>
              <Show when={current().perspective?.mesh_errors?.[0]?.message}>
                {(message) => (
                  <div class="machine-recognition-error" role="alert">
                    <strong>Mesh state could not be read</strong>
                    <span>{message()}</span>
                  </div>
                )}
              </Show>

              <p class="quiet machine-boundary">
                Capabilities, credentials, processes, and live resource pressure stay on this device.
              </p>
            </>
          );
        }}
      </Show>
    </section>
  );
}
