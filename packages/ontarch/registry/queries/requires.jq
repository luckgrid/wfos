# Ids of units that require a capability.  jq --arg cap proto -f requires.jq units.json
.units[] | select((.requires // []) | index($cap)) | .id
