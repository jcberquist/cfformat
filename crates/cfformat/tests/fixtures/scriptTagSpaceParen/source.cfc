// script
cffile (action="write" addnewline="yes" file="#path#" output="aaa");
cfdocument (format="PDF", name="local.test") {
    echo("x");
}
cfloop (query="q") {
    x++;
}
cffile /* before the attributes */ (action="read" file="#path#" variable="x");
cffile // before the attributes
(action="read" file="#path#" variable="x");
