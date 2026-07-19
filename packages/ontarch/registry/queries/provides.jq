# Ids of units that provide a capability.  jq --arg cap metadata.registry -f provides.jq units.json
.units[] | select((.provides // []) | index($cap)) | .id
