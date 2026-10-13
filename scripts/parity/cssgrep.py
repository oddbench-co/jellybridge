import re,sys
css=open('all.css').read()
def rules(text):
    out=[]; depth=0; start=0
    for i,c in enumerate(text):
        if c=='{': depth+=1
        elif c=='}':
            depth-=1
            if depth==0: out.append(text[start:i+1].strip()); start=i+1
    return out
rs=rules(css)
pat=re.compile(sys.argv[1]); limit=int(sys.argv[2]) if len(sys.argv)>2 else 60
n=0
for r in rs:
    if r.startswith('@media') or r.startswith('@supports'):
        head=r[:r.index('{')]
        for sub in rules(r[r.index('{')+1:-1]):
            if pat.search(sub.split('{')[0]):
                print(head.strip(),'::',sub[:300]); n+=1
    elif pat.search(r.split('{')[0]):
        print(r[:300]); n+=1
    if n>=limit: break
