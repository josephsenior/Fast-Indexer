import requests
from pprint import pprint
owner='josephsenior'
repo='Fast-Indexer'
# Use public endpoint to fetch repo info
url=f'https://api.github.com/repos/{owner}/{repo}'
resp=requests.get(url)
print('status',resp.status_code)
try:
    data=resp.json()
    pprint({
        'description':data.get('description'),
        'topics':data.get('topics'),
        'language':data.get('language'),
        'languages_url':data.get('languages_url'),
    })
except Exception as e:
    print('json error',e)
