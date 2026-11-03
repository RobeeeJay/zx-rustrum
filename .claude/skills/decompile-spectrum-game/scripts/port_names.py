#!/usr/bin/env python3
"""Carry routine names from one game to another built on the same engine.

    port_names.py DONE.lst NEW.lst [N]

Both are listings from export_listing (facts on or off). Every label in DONE.lst is
looked for in NEW.lst by the first N instructions after it (default 6) with all
four-digit addresses and names blanked out, since the same routine sits elsewhere
and names other addresses. A label found exactly once is printed as a candidate:

    NEWADDR|name|DONE's comment|from $DONEADDR

These are candidates, not findings. The names usually carry; the comments name the
first game's addresses, values and rules, so read the code before keeping any of
them. A candidate whose name you then give to a second address means one of the two
is wrong.
"""
import re
import sys


def read(path):
    """The instructions in order, and the label each one is under, from a listing."""
    code, labels, label = [], {}, None
    for line in open(path, encoding='utf-8'):
        m = re.match(r'^(\w+):\s*$', line)
        if m:
            label = m.group(1)
            continue
        m = re.match(r'^\$([0-9A-F]{4})  (?:[0-9A-F]{2} )+\s*(.*?)\s*(;.*)?$', line)
        if not m:
            continue
        at = int(m.group(1), 16)
        code.append((at, m.group(2)))
        if label:
            labels[at] = label
            label = None
    return code, labels


def comments(path):
    """The description written above each label."""
    found, label, text = {}, None, []
    for line in open(path, encoding='utf-8'):
        m = re.match(r'^(\w+):\s*$', line)
        if m:
            label, text = m.group(1), []
            continue
        if label and line.startswith('; ') and not line.startswith(('; seen:', '; used by:')):
            text.append(line[2:].strip())
            continue
        if label:
            found[label] = ' '.join(text)
            label = None
    return found


def main():
    done, new = sys.argv[1], sys.argv[2]
    n = int(sys.argv[3]) if len(sys.argv) > 3 else 6
    # Addresses differ between the two games, and a listing shows a name where it has
    # one, so both are blanked: names are lower case and mnemonics upper.
    def blank(text):
        text = re.sub(r'\$[0-9A-F]{4}\b', 'nn', text)
        return re.sub(r'\b[a-z_][a-z0-9_]*\b', 'nn', text)
    code_a, labels_a = read(done)
    code_b, _ = read(new)
    said = comments(done)
    index_a = {at: i for i, (at, _) in enumerate(code_a)}
    where = {}
    for i in range(len(code_b) - n):
        key = ' ; '.join(blank(t) for _, t in code_b[i:i + n])
        where.setdefault(key, []).append(code_b[i][0])
    once = twice = none = 0
    for at, name in sorted(labels_a.items()):
        i = index_a[at]
        key = ' ; '.join(blank(t) for _, t in code_a[i:i + n])
        hits = where.get(key, [])
        if len(hits) == 1:
            once += 1
            print(f"{hits[0]:04X}|{name}|{said.get(name, '')}|from ${at:04X}")
        elif hits:
            twice += 1
        else:
            none += 1
    print(f"{once} found once, {twice} more than once, {none} not found", file=sys.stderr)


main()
