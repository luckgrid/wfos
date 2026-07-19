# Units in a given layer.  jq --arg layer application -f by-layer.jq units.json
.units[] | select(.layer == $layer)
