from libs.heiwa_sdk.celery_config import make_celery

# The specific Celery instance for Cloud HQ
# Usage: celery -A fleets.hub.celery_entry worker --loglevel=info
app = make_celery("heiwa_cloud_hq")

if __name__ == "__main__":
    app.start()
