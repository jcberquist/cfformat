<cffunction name="f">
    <cfargument name="prefix">
    <cfloop item="#arguments.prefix#Item" array="#items#"></cfloop>
    <cfquery name="q#arguments.prefix#">select 1</cfquery>
    <cfquery name=u#arguments.prefix#>select 1</cfquery>
    <cfsavecontent variable="#name#">x</cfsavecontent>
    <cfquery name="q">select 1</cfquery>
    <cfloop index="i" array="#items#"></cfloop>
</cffunction>
