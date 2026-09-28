<cffunction name="report" output="false">
    <cfargument name="rows" type="array">
    <cfset var html = "">
    <cfscript>
        for (var row in arguments.rows) {
            html &= row;
            count = (count ?: 0) + 1;
        }
        rows = [];
    </cfscript>
    <cfset html = trim(html)>
    <cfset out = html>
    <cfreturn out>
</cffunction>
