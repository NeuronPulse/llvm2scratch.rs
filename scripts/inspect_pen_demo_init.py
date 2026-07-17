import json, zipfile
with zipfile.ZipFile('pen_demo.sb3') as z:
    d = json.loads(z.read('Project/project.json'))
blocks = d['targets'][2]['blocks']

# Find !init definition
for bid, b in blocks.items():
    if b.get('mutation',{}).get('proccode') == '!init':
        print('!init def', bid)
        cur = b.get('next')
        for _ in range(20):
            if not cur:
                break
            bb = blocks.get(cur, {})
            print(cur, bb.get('opcode'), bb.get('inputs'), bb.get('fields'))
            if bb.get('opcode') == 'data_replaceitemoflist':
                print('  list input:', bb.get('inputs', {}).get('LIST'))
            cur = bb.get('next')
        break

print('\nLists:')
for name, val in d['targets'][2]['lists'].items():
    print(name, len(val[1]), val[1][:30])
