# Ids of units that depend on a unit, i.e. require a capability that unit provides.
#   jq --arg id panoply -f depends-on.jq units.json
. as $reg
| ($reg.units[] | select(.id == $id) | (.provides // [])) as $caps
| $reg.units[]
| select(.id != $id and ((.requires // []) | any(. as $r | $caps | index($r))))
| .id
