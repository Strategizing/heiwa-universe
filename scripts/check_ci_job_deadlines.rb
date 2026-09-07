#!/usr/bin/env ruby
# frozen_string_literal: true

# A deadline only bounds running jobs, never time spent waiting for an
# unavailable runner. Check runner selection across every workflow, including
# release/certification, while retaining the two CI-specific deadline tiers.

require "yaml"

FAST_LANE_DEADLINE_MINUTES = 1
COMPILE_LANE_DEADLINE_MINUTES = 20
COMPILE_LANES = %w[rust-tests rust-static desktop-app].freeze
# Only standard GitHub-hosted platforms used by this repository are admitted.
# Blacksmith's historical claim receipts expired when it stopped adopting jobs
# on 2026-08-28/29. Probe future candidates in the isolated manual canary first.
# Platform/architecture reference: https://docs.github.com/en/actions/reference/runners/github-hosted-runners
GITHUB_HOSTED_RUNNER_LABELS = %w[
  ubuntu-latest windows-latest windows-2025 macos-latest macos-15 macos-26
].freeze

def workflow_triggers(workflow)
  # Psych parses unquoted `on` as boolean true (YAML 1.1).
  workflow["on"] || workflow[true] || {}
end

def isolated_canary?(path, workflow, jobs)
  triggers = workflow_triggers(workflow)
  probe = jobs["probe"]
  File.basename(path) == "runner-canary.yml" && workflow["name"] == "Runner Canary" &&
    triggers.is_a?(Hash) && triggers.keys == ["workflow_dispatch"] &&
    jobs.keys == ["probe"] && probe.is_a?(Hash) &&
    probe["runs-on"] == '${{ inputs.runner }}' &&
    probe["timeout-minutes"].is_a?(Integer) && probe["timeout-minutes"].between?(1, 5)
end

# Support the two static shapes actually used in this repo: a matrix axis
# (matrix.os) or include records (matrix.runner). Do not guess at expressions
# or dynamic matrices; unresolved selectors must fail before they can queue.
def runner_labels(job)
  runs_on = job["runs-on"]
  runs_on = runs_on.first if runs_on.is_a?(Array) && runs_on.size == 1
  unless runs_on.is_a?(String) && !runs_on.empty?
    raise ArgumentError, "missing or unsupported runs-on"
  end

  match = runs_on.match(/\A\$\{\{\s*matrix\.([a-zA-Z_][a-zA-Z0-9_-]*)\s*\}\}\z/)
  if match
    key = match[1]
    strategy = job["strategy"]
    matrix = strategy.is_a?(Hash) ? strategy["matrix"] : nil
    raise ArgumentError, "unresolved matrix.#{key}" unless matrix.is_a?(Hash)

    axis = matrix[key]
    includes = matrix.fetch("include", [])
    unless (axis.nil? || (axis.is_a?(Array) && !axis.empty?)) && includes.is_a?(Array) &&
           includes.all? { |entry| entry.is_a?(Hash) && entry.key?(key) }
      raise ArgumentError, "unresolved matrix.#{key}"
    end
    labels = Array(axis) + includes.select { |entry| entry.key?(key) }.map { |entry| entry[key] }
    unless !labels.empty? && labels.all? { |label| label.is_a?(String) && !label.empty? && !label.include?("${{") }
      raise ArgumentError, "unresolved matrix.#{key}"
    end
    return labels.uniq
  end

  raise ArgumentError, "unresolved runner expression #{runs_on}" if runs_on.include?("${{")

  [runs_on]
end

paths = ARGV.empty? ? Dir.glob(".github/workflows/*.{yml,yaml}").sort : ARGV
abort "no workflows found" if paths.empty?

failed = false
job_count = 0
paths.each do |path|
  begin
    workflow = YAML.safe_load(File.read(path), aliases: true)
    jobs = workflow.fetch("jobs")
    raise ArgumentError, "workflow jobs must be a nonempty mapping" unless jobs.is_a?(Hash) && !jobs.empty?
  rescue Psych::Exception, IOError, SystemCallError, NoMethodError, KeyError, ArgumentError => error
    warn "#{path}: #{error.message}"
    failed = true
    next
  end

  deadline_violations = []
  runner_violations = []
  action_violations = []
  trigger_violations = []
  ci = workflow["name"] == "Heiwa CI" || File.basename(path) == "ci.yml"
  canary = isolated_canary?(path, workflow, jobs)

  if ci
    triggers = workflow_triggers(workflow)
    pull_request = triggers.is_a?(Hash) ? triggers["pull_request"] : nil
    branches = pull_request.is_a?(Hash) ? Array(pull_request["branches"]) : []
    missing_targets = %w[dev main] - branches
    unless missing_targets.empty?
      trigger_violations << "Heiwa CI pull_request targets must include dev and main " \
                            "(missing: #{missing_targets.join(', ')})"
    end
  end

  jobs.each do |job_id, job|
    job_count += 1
    unless job.is_a?(Hash)
      runner_violations << "#{job_id} (job must be a mapping)"
      next
    end
    # Reusable workflow callers have no runner or deadline. The called local
    # workflow is inspected by the default all-workflow scan.
    if job.key?("uses") && !job.key?("runs-on") && !job.key?("steps")
      unless job["uses"].is_a?(String) && job["uses"].match?(/\A\.\/\.github\/workflows\/[a-zA-Z0-9_-]+\.ya?ml\z/)
        runner_violations << "#{job_id} (runner policy only inspects local reusable workflows)"
      end
      next
    end

    if ci
      deadline = job["timeout-minutes"]
      allowed = COMPILE_LANES.include?(job_id) ? COMPILE_LANE_DEADLINE_MINUTES : FAST_LANE_DEADLINE_MINUTES
      if !deadline.is_a?(Integer) || !deadline.positive?
        deadline_violations << "#{job_id} (no deadline)"
      elsif deadline > allowed
        deadline_violations << "#{job_id} (#{deadline}m > #{allowed}m)"
      end
    end

    unless canary
      begin
        unknown = runner_labels(job) - GITHUB_HOSTED_RUNNER_LABELS
        runner_violations << "#{job_id} (#{unknown.join(', ')})" unless unknown.empty?
      rescue ArgumentError => error
        runner_violations << "#{job_id} (#{error.message})"
      end
    end

    Array(job["steps"]).each do |step|
      action = step.is_a?(Hash) ? step["uses"].to_s : ""
      if action.downcase.start_with?("useblacksmith/")
        action_violations << "#{job_id} (#{action})"
      end
    end
  end

  unless deadline_violations.empty?
    warn "#{path}: every CI job must have a bounded deadline " \
         "(#{FAST_LANE_DEADLINE_MINUTES}m feedback lane, " \
         "#{COMPILE_LANE_DEADLINE_MINUTES}m for #{COMPILE_LANES.join(', ')}); " \
         "missing: #{deadline_violations.join(', ')}"
  end
  unless runner_violations.empty?
    warn "#{path}: every workflow job must target an approved GitHub-hosted runner " \
         "(#{GITHUB_HOSTED_RUNNER_LABELS.join(', ')}); unproven: #{runner_violations.join(', ')}"
  end
  action_violations.each { |violation| warn "#{path}: retired runner action: #{violation}" }
  trigger_violations.each { |violation| warn "#{path}: #{violation}" }
  failed ||= [deadline_violations, runner_violations, action_violations, trigger_violations].any? { |items| !items.empty? }
end

exit 1 if failed

puts "CI deadlines and workflow runners passed (#{job_count} jobs, #{paths.size} workflows)."
