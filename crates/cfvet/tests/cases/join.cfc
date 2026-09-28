component {
  function c() {
    if (a) { var x = 1; }
    else if (b) { var x = 2; }
    else { var x = 3; }
    x = 4;
    if (a) { var y = 1; }
    else if (b) { y = 2; }
    else { var y = 3; }
    y = 4;
    if (a) var z = 1;
    else { if (b) var z = 2; else var z = 3; }
    z = 4;
    switch (a) {
      case 1: case 2: var w = 1; break;
      default: var w = 2;
    }
    w = 3;
    switch (a) { case 1: var v = 1; break; case 2: var v = 2; }
    v = 3;
  }
}
