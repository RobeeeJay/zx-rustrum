#!/usr/bin/env python3
"""Turn the ZXDB dump into the list of games the lineage tools read.

ZXDB (https://github.com/zxdb/ZXDB) is Spectrum Computing's database of every
title, who wrote it and who published it. It is somebody else's work and is
not copied into this repository; this reads its MySQL dump and writes one line
a title, tab-separated, into the preferences directory:

    python3 tools/zxdb-games.py ZXDB_mysql.sql.zip > zxdb-games.txt

    id  title  year  publisher  authors

The authors are `Name (Code, Music); Name (In-game Graphics)`, in ZXDB's own
order and with ZXDB's own words for the roles. The year and the publisher are
those of the original release; a re-release on a budget label is a later
release of the same entry and is left out.

The dump is plain INSERT statements with a row a line, so it is read a line at
a time rather than loaded into a database.
"""

import re
import sys
import zipfile
from collections import defaultdict

INSERT = re.compile(r"^INSERT INTO `(\w+)` \(([^)]*)\) VALUES")
WANTED = {"entries", "labels", "authors", "roles", "roletypes", "releases", "publishers"}


def values(row):
    """The fields of one `(1, 'a', NULL),` row, as Python values."""
    fields, i, n = [], row.index("(") + 1, len(row)
    while i < n:
        c = row[i]
        if c in " ,":
            i += 1
        elif c == ")":
            break
        elif c == "'":
            i += 1
            out = []
            while i < n:
                c = row[i]
                if c == "\\":
                    out.append({"n": "\n", "t": "\t", "0": ""}.get(row[i + 1], row[i + 1]))
                    i += 2
                elif c == "'" and i + 1 < n and row[i + 1] == "'":
                    out.append("'")
                    i += 2
                elif c == "'":
                    i += 1
                    break
                else:
                    out.append(c)
                    i += 1
            fields.append("".join(out))
        else:
            j = i
            while j < n and row[j] not in ",)":
                j += 1
            word = row[i:j].strip()
            fields.append(None if word == "NULL" else word)
            i = j
    return fields


def tables(lines):
    """Rows of the tables that matter, as dicts by column name."""
    table, columns = None, []
    for line in lines:
        found = INSERT.match(line)
        if found:
            table = found.group(1) if found.group(1) in WANTED else None
            columns = [c.strip(" `") for c in found.group(2).split(",")]
            continue
        if table and line.startswith("\t("):
            yield table, dict(zip(columns, values(line)))
            if line.rstrip().endswith(";"):
                table = None
        elif not line.startswith("\t"):
            table = None


def open_dump(path):
    if path.endswith(".zip"):
        archive = zipfile.ZipFile(path)
        name = next(n for n in archive.namelist() if n.endswith(".sql"))
        return (line.decode("utf-8", "replace") for line in archive.open(name))
    return open(path, encoding="utf-8", errors="replace")


def main(path):
    titles, labels, roletypes = {}, {}, {}
    authors = defaultdict(list)
    roles = defaultdict(list)
    years, publishers = {}, defaultdict(list)
    for table, row in tables(open_dump(path)):
        if table == "entries":
            titles[row["id"]] = row["title"]
        elif table == "labels":
            labels[row["id"]] = row["name"]
        elif table == "roletypes":
            roletypes[row["id"]] = row["text"]
        elif table == "authors":
            authors[row["entry_id"]].append((int(row["author_seq"]), row["label_id"]))
        elif table == "roles":
            roles[(row["entry_id"], row["label_id"])].append(row["roletype_id"])
        elif table == "releases" and row["release_seq"] == "0":
            years[row["entry_id"]] = row["release_year"] or ""
        elif table == "publishers" and row["release_seq"] == "0":
            publishers[row["entry_id"]].append((int(row["publisher_seq"]), row["label_id"]))

    print("# From ZXDB, https://github.com/zxdb/ZXDB, by tools/zxdb-games.py.")
    print("# id\ttitle\tyear\tpublisher\tauthors")
    for entry, title in sorted(titles.items(), key=lambda e: int(e[0])):
        who = []
        for _, label in sorted(authors.get(entry, [])):
            jobs = sorted(roletypes.get(r, r) for r in roles.get((entry, label), []))
            name = labels.get(label, "?")
            who.append(f"{name} ({', '.join(jobs)})" if jobs else name)
        by = " / ".join(labels.get(l, "?") for _, l in sorted(publishers.get(entry, [])))
        clean = lambda s: s.replace("\t", " ").replace("\n", " ")
        print("\t".join([entry, clean(title), years.get(entry, ""), clean(by), clean("; ".join(who))]))


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit("usage: zxdb-games.py ZXDB_mysql.sql[.zip] > zxdb-games.txt")
    main(sys.argv[1])
