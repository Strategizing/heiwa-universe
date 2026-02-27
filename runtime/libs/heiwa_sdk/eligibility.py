import json
import datetime


def compute_eligibility(nodes, request):
    """
    Compute eligibility for a work request against a list of nodes.

    Args:
        nodes: List of node dicts (from DB).
        request: Dict with keys:
            - work_type (str)
            - requirements (dict):
                - needs_gpu (bool)
                - needs_docker (bool)
                - needs_models (list[str])
                - allowed_tags (list[str])
                - blocked_tags (list[str])
                - min_agent_version (str)
                - max_concurrency (int)

    Returns:
        dict: {
            "eligible": [{"node_id": str, "score": int, "reasons": list[str]}],
            "ineligible": [{"node_id": str, "reasons": list[str]}]
        }
    """
    eligible = []
    ineligible = []

    reqs = request.get("requirements", {})
    work_type = request.get("work_type", "UNKNOWN")

    # Constants
    PROD_REQUIRED_TAG = "prod-approved"

    # Helper for version check (simple string compare for now, ideally semver)
    def version_ge(v_node, v_req):
        if not v_node:
            return False
        return v_node >= v_req

    now = datetime.datetime.now(datetime.timezone.utc)

    for node in nodes:
        reasons = []
        is_eligible = True
        score = 50  # Base score

        # 0. Status Check
        if node.get("status") != "ONLINE":
            is_eligible = False
            reasons.append(f"status={node.get('status')}")

        # Parse capabilities/tags
        caps = node.get("capabilities_json", "{}")
        if not caps:
            caps = "{}"  # Handle None or empty string
        # If it's a dict already (unlikely from raw text db), handle it
        if isinstance(caps, dict):
            pass
        else:
            try:
                caps = json.loads(caps)
            except:
                caps = {}

        tags = node.get("tags_json", "[]")
        if not tags:
            tags = "[]"
        if isinstance(tags, list):
            pass
        else:
            try:
                tags = json.loads(tags)
            except:
                tags = []

        version = node.get("agent_version", "")

        # 1. Capabilities Check
        if reqs.get("needs_gpu") and not caps.get("has_gpu"):
            is_eligible = False
            reasons.append("missing GPU")

        if reqs.get("needs_docker") and "DOCKER" not in caps.get("can_run", []):
            is_eligible = False
            reasons.append("missing DOCKER")

        if reqs.get("needs_models"):
            supported_models = set(caps.get("models", []))
            missing_models = [
                m for m in reqs["needs_models"] if m not in supported_models
            ]
            if missing_models:
                is_eligible = False
                reasons.append(f"missing models: {missing_models}")

        # 2. Tags Check
        if reqs.get("allowed_tags"):
            # Node MUST have at least one allowed tag? Or Node tags must match allowed?
            # Typically "allowed_tags" in request means "Only run on nodes with these tags".
            # If request.allowed_tags = ["forge"], node MUST have "forge".
            # If empty, any node allowed.
            required = set(reqs["allowed_tags"])
            node_tags = set(tags)
            if not node_tags.intersection(required):
                is_eligible = False
                reasons.append("missing allowed_tag")

        if reqs.get("blocked_tags"):
            blocked = set(reqs["blocked_tags"])
            node_tags = set(tags)
            if node_tags.intersection(blocked):
                is_eligible = False
                reasons.append("has blocked_tag")

        # Prod Check (Implicit Rule)
        if work_type == "DEPLOY" and PROD_REQUIRED_TAG not in tags:
            # Just a scoring penalty or hard block?
            # "Prefer tag:prod-approved (if prod work)".
            # User prompt says "Prefer tag:prod-approved", implying scoring, but also "Ranking rule".
            # Actually "Only run on prod nodes" might be safer.
            # Let's handle it via scoring boost for now unless explicit requirement added.
            pass

        # 3. Version Check
        if reqs.get("min_agent_version"):
            if not version_ge(version, reqs["min_agent_version"]):
                is_eligible = False
                reasons.append(f"version {version} < {reqs['min_agent_version']}")

        # Scoring
        if is_eligible:
            # Positive Reasons
            reasons.append("ONLINE")

            # Tag Boost
            if reqs.get("allowed_tags") and set(tags).intersection(
                set(reqs["allowed_tags"])
            ):
                score += 10
                reasons.append("tag match")

            if work_type == "DEPLOY" and PROD_REQUIRED_TAG in tags:
                score += 20
                reasons.append("prod-approved")

            # Recency Boost
            if node.get("last_heartbeat_at"):
                try:
                    last_hb = datetime.datetime.fromisoformat(node["last_heartbeat_at"])
                    if last_hb.tzinfo is None:
                        last_hb = last_hb.replace(tzinfo=datetime.timezone.utc)
                    age_seconds = (now - last_hb).total_seconds()
                    if age_seconds < 60:
                        score += 5
                        reasons.append("recent heartbeat")
                except:
                    pass

            # Concurrency Boost
            # We don't track active jobs perfectly yet, but we prefer higher max_concurrency
            mc = node.get("max_concurrency", 1)
            score += mc

            eligible.append(
                {"node_id": node["node_id"], "score": score, "reasons": reasons}
            )
        else:
            ineligible.append({"node_id": node["node_id"], "reasons": reasons})

    # Sort eligible by score desc
    eligible.sort(key=lambda x: x["score"], reverse=True)

    return {"eligible": eligible, "ineligible": ineligible}
