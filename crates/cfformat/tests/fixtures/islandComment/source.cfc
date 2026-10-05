<style>
/*
 * A header whose lines start with a star.
 */
body { margin: 0; }
/* a comment
   whose lines do not */
.a { color: red;
/* inside a rule:
     indented further
   and less */
background: url( a.png );
content: "/* not a comment";
}
</style>
<div>
    <style>
        /*
         * Written where a run leaves it.
         */
        .b { color: blue; }
        /* raw,
           aligned by hand */
    </style>
    <script type="application/json">
    {
    /* raw
       in JSON */
    "a": 1,
    /*
     * starred in JSON
     */
    "b": [1, 2]
    }
    </script>
    <cfoutput>
        <style>
        /* #title#
           holds an expression */
        .#cls# { width: #w#px; }
        </style>
    </cfoutput>
</div>
