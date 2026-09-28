component {
function a() {
    try {
        var x = 1;
    } catch(any e) {
        // pass
    }
    x = 2;
    return x;
}

function b() {
    try {
        var t = 1;
    } catch (any e) {
        t = 2;
        var c = 1;
    } finally {
        t = 3;
    }
    t = 4;
    c = 2;
}
}
