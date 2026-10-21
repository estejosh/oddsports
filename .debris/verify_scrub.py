import sqlite3, sys, glob
files = ["data/oddsports.sqlite"] + glob.glob("data/oddsports.sqlite.pre-scrub-*")
for f in files:
    d = sqlite3.connect(f)
    q = lambda s: d.execute(s).fetchone()[0]
    print(f,
          "| slates w/ betchu:", q("select count(*) from slates where lower(json) like '%betchu%'"),
          "| graded w/ betchu or affiliated:", q("select count(*) from graded_picks where lower(reveal_body) like '%betchu%' or lower(reveal_body) like '%affiliated%'"),
          "| graded rows:", q("select count(*) from graded_picks"),
          "| units:", q("select round(sum(units_delta),2) from graded_picks"),
          "| wins:", q("select count(*) from graded_picks where result='win'"))
