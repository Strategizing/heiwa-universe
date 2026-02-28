terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "~> 4.0"
    }
  }
}

provider "cloudflare" {
  # CLOUDFLARE_API_TOKEN environment variable must be set.
}

variable "zone_id" {
  description = "The Cloudflare Zone ID for heiwa.ltd"
  type        = string
}

variable "railway_cname_target" {
  description = "The Railway domain target for API/Auth"
  type        = string
  default     = "heiwa-cloud-hq-brain.up.railway.app"
}

variable "pages_cname_target" {
  description = "The Cloudflare Pages domain target for status/docs"
  type        = string
  default     = "heiwa-clients.pages.dev"
}

# -------------------------------------------------------------------------
# DNS Records
# -------------------------------------------------------------------------

resource "cloudflare_record" "status" {
  zone_id = var.zone_id
  name    = "status"
  value   = var.pages_cname_target
  type    = "CNAME"
  proxied = true
}

resource "cloudflare_record" "docs" {
  zone_id = var.zone_id
  name    = "docs"
  value   = var.pages_cname_target
  type    = "CNAME"
  proxied = true
}

resource "cloudflare_record" "auth" {
  zone_id = var.zone_id
  name    = "auth"
  value   = var.railway_cname_target
  type    = "CNAME"
  proxied = true
}

resource "cloudflare_record" "api" {
  zone_id = var.zone_id
  name    = "api"
  value   = var.railway_cname_target
  type    = "CNAME"
  proxied = true
}

# -------------------------------------------------------------------------
# WAF / Edge Rules
# -------------------------------------------------------------------------

resource "cloudflare_ruleset" "heiwa_waf" {
  zone_id     = var.zone_id
  name        = "Heiwa Core Security WAF"
  description = "Protect API and Auth endpoints"
  kind        = "zone"
  phase       = "http_request_firewall_custom"

  rules {
    action = "block"
    expression = "(http.request.uri.path contains \"/admin\" and not ip.src in {127.0.0.1})"
    description = "Block /admin paths unless local (example placeholder)"
    enabled = true
  }
}
