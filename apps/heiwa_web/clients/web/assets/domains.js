const DOMAIN_MANIFEST_URL = "./assets/domains.bootstrap.json";

function stateBadgeClass(state) {
  if (state === "active") return "ok";
  if (state === "planned") return "warn";
  return "warn";
}

function humanPlatform(platform) {
  if (!platform) return "Domain routing data unavailable.";
  return `DNS on ${platform.dns}, public web on ${platform.public_web}, and control plane on ${platform.control_plane}.`;
}

function renderDomainCards(domains) {
  const container = document.getElementById("domains-list");
  container.innerHTML = "";

  domains.forEach((domain) => {
    const state = domain.state || "planned";
    const card = document.createElement("article");
    card.className = "panel";
    card.innerHTML = `
      <div class="status-card-head">
        <h2 class="mono">${domain.host}</h2>
        <span class="status-badge ${stateBadgeClass(state)}">${state}</span>
      </div>
      <p>${domain.purpose || "No purpose declared."}</p>
      <div class="domain-kv">
        <div><span>Target</span><strong>${domain.target || "-"}</strong></div>
        <div><span>Health path</span><strong>${domain.health_path || "-"}</strong></div>
      </div>
    `;
    container.appendChild(card);
  });
}

function renderDnsRecords(records) {
  const body = document.getElementById("dns-records");
  body.innerHTML = "";
  records.forEach((record) => {
    const row = document.createElement("tr");
    row.innerHTML = `
      <td>${record.type || "-"}</td>
      <td class="mono">${record.name || "-"}</td>
      <td class="mono">${record.value || "-"}</td>
      <td>${String(record.proxy ?? "-")}</td>
      <td>${record.ttl || "-"}</td>
    `;
    body.appendChild(row);
  });
}

function renderSteps(steps) {
  const list = document.getElementById("bootstrap-steps");
  list.innerHTML = "";
  steps.forEach((step) => {
    const item = document.createElement("li");
    item.textContent = step;
    list.appendChild(item);
  });
}

function renderManifest(manifest) {
  document.getElementById("manifest-source").textContent =
    manifest.generated_from || DOMAIN_MANIFEST_URL;
  document.getElementById("root-domain").textContent = manifest.root_domain || "heiwa.ltd";
  document.getElementById("platform-summary").textContent = humanPlatform(manifest.platform);
  document.getElementById("platform-dns").textContent = manifest.platform?.dns || "-";
  document.getElementById("platform-web").textContent = manifest.platform?.public_web || "-";
  document.getElementById("platform-control").textContent = manifest.platform?.control_plane || "-";

  renderDomainCards(manifest.domains || []);
  renderDnsRecords(manifest.dns_records || []);
  renderSteps(manifest.bootstrap_steps || []);
}

async function refreshDomains() {
  const button = document.getElementById("refresh-domains");
  button.disabled = true;
  button.textContent = "Refreshing...";
  try {
    const response = await fetch(DOMAIN_MANIFEST_URL, { cache: "no-store" });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const manifest = await response.json();
    renderManifest(manifest);
  } catch (error) {
    const container = document.getElementById("domains-list");
    container.innerHTML = `<article class="panel"><h2>Manifest unavailable</h2><p class="muted mono">${String(error)}</p></article>`;
  } finally {
    button.disabled = false;
    button.textContent = "Refresh";
  }
}

document.addEventListener("DOMContentLoaded", () => {
  document.getElementById("refresh-domains")?.addEventListener("click", refreshDomains);
  refreshDomains();
});
