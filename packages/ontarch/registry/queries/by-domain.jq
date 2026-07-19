# Units in a given domain.  jq --arg domain luckgrid -f by-domain.jq units.json
.units[] | select(.domain == $domain)
