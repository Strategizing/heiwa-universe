import os
import sys
from celery import Celery

def make_celery(app_name: str = "heiwa_worker") -> Celery:
    """
    Factory to create a configured Celery instance.
    Enforces 'Config-as-Code' via explicit settings.
    """
    redis_url = os.getenv("REDIS_URL")
    
    if not redis_url:
        print("[CRITICAL] REDIS_URL not set. Celery cannot start.")
        # We don't exit here to allow import for diverse environments, 
        # but the worker will fail to connect.
    
    celery_app = Celery(app_name, broker=redis_url, backend=redis_url)
    
    # HEIWA DOCTRINE: Secure Serialization
    # We explicitly disable pickle to prevent RCE from untrusted messages.
    celery_app.conf.update(
        task_serializer="json",
        accept_content=["json"],  # Ignore other content
        result_serializer="json",
        timezone="UTC",
        enable_utc=True,
        # Heartbeat for node liveness
        worker_send_task_events=True,
        task_send_sent_event=True,
        # Retry behavior
        broker_connection_retry_on_startup=True,
    )
    
    return celery_app