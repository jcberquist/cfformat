<cfscript>
x = 1;

// `import` with a quoted path.
import "java.lang.String";
import java.lang.Integer;

// A comment between the path and the semicolon stays.
import java.lang.Long
// own
;
import java.util.* /* same line */;
import java.lang.Byte // same line
;
import java.io.* // star path
;
import java.lang.Short /* block */ // line
// own
;
function f() {
    import java.lang.Double // indented
    ;
}

// Comments between the keyword and the path stay.
import /* keep me */ java.util.Map;
import // keep me
    java.util.List;
import
// own line
java.util.Set;
import /* a */ "java.lang.Math" /* b */ ;
</cfscript>
