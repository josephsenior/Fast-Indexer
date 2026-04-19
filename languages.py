import requests
owner='josephsenior'
repo='Fast-Indexer'
url=f'https://api.github.com/repos/{owner}/{repo}/languages'
resp=requests.get(url)
print(resp.status_code)
print(resp.json())
