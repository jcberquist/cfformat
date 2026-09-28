<cfscript>
test = new component {
  function a() {
    x = 1;
    var x = 2;
    return x;
  }

  function b() {
    writeDump(variables);
  }
};

writeDump(test.a());
test.b();
</cfscript>
