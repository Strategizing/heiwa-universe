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

resource "cloudflare_record" "root" {
  zone_id = var.zone_id
  name    = "@"
  value   = var.pages_cname_target
  type    = "CNAME"
  proxied = true
}

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
    description = "Block /admin paths unless local"
    enabled = true
  }

  rules {
    action = "block"
    expression = "(http.request.host eq \"api.heiwa.ltd\" or http.request.host eq \"auth.heiwa.ltd\") and not cf.bot_management.verified_bot"
    description = "Challenge unverified bots hitting compute endpoints"
    enabled = true
  }
}

resource "cloudflare_ruleset" "rate_limiting" {
  zone_id     = var.zone_id
  name        = "Heiwa API Rate Limiting"
  description = "Prevent abuse of Cloud HQ compute"
  kind        = "zone"
  phase       = "http_ratelimit"

  rules {
    action = "block"
    expression = "(http.request.host eq \"api.heiwa.ltd\" or http.request.host eq \"auth.heiwa.ltd\")"
    description = "Rate limit API/Auth to 100 req per minute per IP"
    enabled = true
    action_parameters {
      response {
        status_code = 429
        content = "{\"error\": \"Rate limit exceeded. Too many requests to Heiwa Swarm API.\"}"
        content_type = "application/json"
      }
    }
    ratelimit {
      characteristics = ["ip.src"]
      period          = 60
      requests_per_period = 100
      mitigation_timeout  = 300 # 5 minutes block
    }
  }
}
