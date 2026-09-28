// script
for (var case in arguments.cases) { out = listAppend(out, case.label); }
for (var switch in ["a"]) { out &= switch; }
for (case in ["p"]) { out &= case; }
var case = 1;
final var default = 2;
static switch = 3;
case.x = 4;
switch (a) {
    case 1:
        x = case;
        break;
    default:
        y();
}
