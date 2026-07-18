def utf16_units:
  explode
  | map(if . < 65536 then [.] else (. - 65536) as $n | [55296 + (($n / 1024) | floor), 56320 + ($n % 1024)] end)
  | add;

def jcs_number:
  if . == 0 then "0"
  else tostring
    | sub("E"; "e")
    | sub("e(-?)0+([1-9])"; "e\(.captures[0].string)\(.captures[1].string)")
  end;

def jcs:
  if type == "null" or type == "boolean" then tostring
  elif type == "number" then jcs_number
  elif type == "string" then @json
  elif type == "array" then "[" + (map(jcs) | join(",")) + "]"
  elif type == "object" then
    "{" + (to_entries | sort_by(.key | utf16_units) | map((.key | @json) + ":" + (.value | jcs)) | join(",")) + "}"
  else error("unsupported JSON type")
  end;

.[]
| (.input | fromjson | jcs) as $actual
| if $actual == .canonical then .name else error("canonical mismatch for \(.name): \($actual)") end
