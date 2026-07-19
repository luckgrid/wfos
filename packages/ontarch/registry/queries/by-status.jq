# Units with a given status.  jq --arg status planned -f by-status.jq units.json
.units[] | select(.status == $status)
