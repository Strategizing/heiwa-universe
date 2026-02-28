import requests
import json

url = 'http://100.116.86.118:8000/proposals/claim'
headers = {'X-Auth-Token': 'test-token', 'Content-Type': 'application/json'}
data = {'node_id': 'workstation', 'max_items': 1}

try:
    resp = requests.post(url, headers=headers, json=data)
    print(f'Status: {resp.status_code}')
    print(f'Body: {resp.text}')
except Exception as e:
    print(f'Error: {e}')
