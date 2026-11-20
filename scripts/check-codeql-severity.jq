# CodeQL attaches security-severity to the rule, while results reference its ID.
[
  .runs[] |
  INDEX((.tool.driver, .tool.extensions[]?) | .rules[]?; .id) as $rules |
  .results[] |
  .ruleId as $id |
  ($rules[$id] // error("Unknown CodeQL rule: " + $id)) as $rule |
  ($rule.properties["security-severity"] // 0 | tonumber) as $score |
  select($score >= 7) |
  {rule: $id, security_severity: $score}
] | if length == 0 then true else error("Blocking CodeQL findings: " + tojson) end
