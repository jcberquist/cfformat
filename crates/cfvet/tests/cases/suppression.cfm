<cffunction name="f">
    <cfset a = 1> <!--- cfvet-ignore --->
    <!--- cfvet-ignore: set by the caller's include --->
    <cfset b = 2>
    <cfset c = 3>
    <cfscript>
        d = 4; // cfvet-ignore
        // cfvet-ignore
        e = 5;
        /* cfvet-ignore */ g = 6;
        /**
         * cfvet-ignore
         */
        h = 7;

        i = 8;
    </cfscript>
</cffunction>
