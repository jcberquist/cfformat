// script
try {
    x();
} catch (local.exp) {
    y = local.exp.message;
} catch (any e) {
}
try {} catch ( variables.a.b ) {}
try {} catch ("java.lang.Exception" var e) {}
