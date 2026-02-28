from enum import Enum

class Subject(str, Enum):
    """
    The NATS Subject Topology for Heiwa Swarm.
    Format: heiwa.<domain>.<action>
    """
    # Core Orchestration
    CORE_REQUEST = "heiwa.core.request"        # User/API -> Spine (Do this for me)
    DISPATCH_TASK = "heiwa.core.dispatch"      # Spine -> Node (Worker, execute this)
    TASK_NEW = "heiwa.tasks.new"               # Discord/Gateway -> Proposal agents
    TASK_INGRESS = "heiwa.tasks.ingress"       # Discord/Gateway -> Planner ingress
    TASK_PLAN_REQUEST = "heiwa.tasks.plan.request"
    TASK_PLAN_RESULT = "heiwa.tasks.plan.result"
    TASK_EXEC_REQUEST_CODE = "heiwa.tasks.exec.request.code"
    TASK_EXEC_REQUEST_RESEARCH = "heiwa.tasks.exec.request.research"
    TASK_EXEC_REQUEST_AUTOMATION = "heiwa.tasks.exec.request.automation"
    TASK_EXEC_REQUEST_OPERATE = "heiwa.tasks.exec.request.operate"
    TASK_EXEC_RESULT = "heiwa.tasks.exec.result"
    TASK_APPROVAL_REQUEST = "heiwa.tasks.approval.request"
    TASK_APPROVAL_DECISION = "heiwa.tasks.approval.decision"
    TASK_STATUS = "heiwa.tasks.status"
    NODE_HEARTBEAT = "heiwa.node.heartbeat"    # Node -> Orchestrator (I'm alive)
    NODE_REGISTER = "heiwa.node.register"      # Node -> Orchestrator (Hello, I'm new)

    # V2 Mesh Protocol (Decentralized Blackboard)
    MESH_CAPABILITY_BROADCAST = "heiwa.mesh.capability.broadcast" # Agent -> Mesh (Here are my tools)
    MESH_TASK_BID = "heiwa.mesh.task.bid"                         # Agent -> Mesh (I can do this task)
    MESH_TASK_CLAIM = "heiwa.mesh.task.claim"                     # Orchestrator -> Agent (You won this task)

    # Logs & Telemetry
    LOG_INFO = "heiwa.log.info"
    LOG_ERROR = "heiwa.log.error"
    LOG_THOUGHT = "heiwa.log.thought"

    # Specific Agent Channels (Examples)
    MARKET_UPDATE = "heiwa.market.update"
    SCRAPE_RESULT = "heiwa.scrape.result"

    def __str__(self):
        return self.value

class Payload:
    """Standardized Payload Keys"""
    SENDER_ID = "sender_id"
    TIMESTAMP = "timestamp"
    TYPE = "type"
    DATA = "data"
    ERROR = "error"
