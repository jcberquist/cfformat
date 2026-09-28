component {
  function c() {
    if (a) var y = 1;
    else { var z = 1; }
    switch (a) { case 1: var w = 1; break; default: w = 2; }
    try { var t = 1; }
    catch (any e) { t = 2; }
    finally { t = 3; }
    y = 2;
    z = 2;
    e = 2;
  }
}
