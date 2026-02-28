const DEFAULT_ENDPOINTS = [
  "https://heiwa-cloud-hq-brain.up.railway.app/health",
  "https://api.heiwa.ltd/health",
];

function getConfiguredEndpoints() {
  const params = new URLSearchParams(window.location.search);
  const queryEndpoints = params.getAll("endpoint").map((v) => v.trim()).filter(Boolean);
  if (queryEndpoints.length) return queryEndpoints;
  if (Array.isArray(window.HEIWA_STATUS_ENDPOINTS) && window.HEIWA_STATUS_ENDPOINTS.length) {
    return window.HEIWA_STATUS_ENDPOINTS;
  }
  return DEFAULT_ENDPOINTS;
}

async function probe(url) {
  const started = performance.now();
  try {
    const res = await fetch(url, { method: "GET", cache: "no-store" });
    const elapsed = Math.round(performance.now() - started);
    let payload = null;
    try {
      payload = await res.json();
    } catch {
      payload = { note: "Non-JSON response" };
    }
    return {
      url,
      ok: res.ok,
      status: res.status,
      durationMs: elapsed,
      payload,
    };
  } catch (error) {
    return {
      url,
      ok: false,
      status: null,
      durationMs: Math.round(performance.now() - started),
      error: String(error),
    };
  }
}

function cardStatus(result) {
  if (result.ok) return { label: "healthy", cls: "ok" };
  if (result.status && result.status < 500) return { label: "warning", cls: "warn" };
  return { label: "unhealthy", cls: "fail" };
}

function renderSummary(results) {
  const healthy = results.filter((r) => r.ok).length;
  const total = results.length;
  const warns = total - healthy;
  const allHealthy = healthy === total && total > 0;

  document.getElementById("healthy-count").textContent = String(healthy);
  document.getElementById("warn-count").textContent = String(warns);
  document.getElementById("total-count").textContent = String(total);

  const headline = document.querySelector("#status-summary h2");
  const text = document.getElementById("status-summary-text");
  if (allHealthy) {
    headline.textContent = "Platform checks healthy";
    text.textContent = "All configured endpoints returned success responses.";
  } else if (healthy > 0) {
    headline.textContent = "Partial health";
    text.textContent = "Some endpoints are healthy; review warnings below.";
  } else {
    headline.textContent = "Health checks need attention";
    text.textContent = "No configured endpoints returned a healthy response.";
  }
}

function renderResults(results) {
  const list = document.getElementById("status-list");
  list.innerHTML = "";

  results.forEach((result) => {
    const state = cardStatus(result);
    const card = document.createElement("article");
    card.className = "panel";

    const prettyPayload = JSON.stringify(
      result.error ? { error: result.error } : result.payload,
      null,
      2
    );

    card.innerHTML = `
      <div class="status-card-head">
        <h2 class="mono">${result.url}</h2>
        <span class="status-badge ${state.cls}">${state.label}</span>
      </div>
      <p class="muted">HTTP ${result.status ?? "ERR"} · ${result.durationMs}ms</p>
      <pre class="status-payload">${prettyPayload}</pre>
    `;
    list.appendChild(card);
  });
}

async function refreshStatus() {
  const button = document.getElementById("refresh-status");
  button.disabled = true;
  button.textContent = "Refreshing…";
  try {
    const endpoints = getConfiguredEndpoints();
    const results = await Promise.all(endpoints.map(probe));
    renderSummary(results);
    renderResults(results);
  } finally {
    button.disabled = false;
    button.textContent = "Refresh";
  }
}

document.addEventListener("DOMContentLoaded", () => {
  document.getElementById("refresh-status")?.addEventListener("click", refreshStatus);
  refreshStatus();
});
