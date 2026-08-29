#!/usr/bin/env ruby
# frozen_string_literal: true

# Guards the two properties that decide whether a CI job can actually finish:
#
#   1. a hard deadline, and
#   2. a runner label some runner will actually claim.
#
# `timeout-minutes` bounds a *running* job. It does NOT bound queue time: a job
# asking for a label no runner provides sits queued until GitHub's 24h ceiling.
# That is how `blacksmith-16vcpu-ubuntu-2404` held the promotion gate open for
# six hours with a one-minute deadline on the job. Both properties, or the gate
# is decorative.
#
# Deadlines come in two tiers: the feedback lane stays sub-minute, and the Rust
# promotion lane compiles so it gets a larger — but still hard — cap.
#
# To add a runner label here, first prove it claims a job in this repository.

require "yaml"

FAST_LANE_DEADLINE_MINUTES = 1
COMPILE_LANE_DEADLINE_MINUTES = 20
# desktop-app compiles the shipped app crate and its release build, so it
# belongs in the compile lane rather than the sub-minute feedback lane.
COMPILE_LANES = %w[rust-tests rust-static desktop-app].freeze
# Both labels have claim evidence. `blacksmith-4vcpu-ubuntu-2404` claimed every
# job in CI run 31914594311 (PR #68) within ~10s. Required PR CI moved back to
# `ubuntu-latest` on 2026-08-29 after Blacksmith stopped adopting jobs across
# independent PRs; keep the old label allowlisted only for a deliberate canary.
PROVEN_RUNNER_LABELS = %w[ubuntu-latest blacksmith-4vcpu-ubuntu-2404].freeze

path = ARGV.fetch(0, ".github/workflows/ci.yml")
workflow = YAML.safe_load(File.read(path), aliases: true)
jobs = workflow.fetch("jobs")

deadline_violations = []
runner_violations = []
trigger_violations = []

if workflow["name"] == "Heiwa CI"
  # Psych follows YAML 1.1 and parses the unquoted GitHub key `on` as boolean
  # true, while other parsers retain it as a string. Accept both shapes.
  triggers = workflow["on"] || workflow[true] || {}
  pull_request = triggers.is_a?(Hash) ? triggers["pull_request"] : nil
  branches = pull_request.is_a?(Hash) ? Array(pull_request["branches"]) : []
  required_targets = %w[dev main]
  missing_targets = required_targets - branches
  unless missing_targets.empty?
    trigger_violations << "Heiwa CI pull_request targets must include dev and main " \
                          "(missing: #{missing_targets.join(', ')})"
  end
end

jobs.each do |job_id, job|
  deadline = job.is_a?(Hash) ? job["timeout-minutes"] : nil
  allowed = COMPILE_LANES.include?(job_id) ? COMPILE_LANE_DEADLINE_MINUTES : FAST_LANE_DEADLINE_MINUTES

  if !deadline.is_a?(Integer) || !deadline.positive?
    deadline_violations << "#{job_id} (no deadline)"
  elsif deadline > allowed
    deadline_violations << "#{job_id} (#{deadline}m > #{allowed}m)"
  end

  runs_on = job.is_a?(Hash) ? job["runs-on"] : nil
  labels = Array(runs_on).flatten
  next if labels.empty?

  unknown = labels.reject { |label| PROVEN_RUNNER_LABELS.include?(label) }
  runner_violations << "#{job_id} (#{unknown.join(', ')})" unless unknown.empty?
end

failed = false

unless deadline_violations.empty?
  warn "every CI job must have a bounded deadline " \
       "(#{FAST_LANE_DEADLINE_MINUTES}m feedback lane, " \
       "#{COMPILE_LANE_DEADLINE_MINUTES}m for #{COMPILE_LANES.join(', ')}); " \
       "missing: #{deadline_violations.join(', ')}"
  failed = true
end

unless runner_violations.empty?
  warn "every CI job must target a runner label proven to claim jobs in this " \
       "repository (#{PROVEN_RUNNER_LABELS.join(', ')}); unproven: " \
       "#{runner_violations.join(', ')}"
  failed = true
end

unless trigger_violations.empty?
  trigger_violations.each { |violation| warn violation }
  failed = true
end

exit 1 if failed

puts "Every CI job has a bounded deadline and a proven runner (#{jobs.size} jobs)."
