# Units of a given kind.  jq --arg kind workspace -f by-kind.jq units.json
.units[] | select(.kind == $kind)
