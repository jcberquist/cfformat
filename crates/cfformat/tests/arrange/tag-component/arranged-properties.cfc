<cfcomponent displayname="Widget" output="false">

    <cfproperty name="alpha" type="string">
    <cfproperty name="zeta" type="string">

    <cfscript>
        variables.ready = true;
        function inScriptB() {}
        function inScriptA() {}
    </cfscript>

    <cffunction name="Init" access="public" returntype="any" output="false">
        <cfargument name="options" type="struct" default="#structNew()#">
        <cfreturn this>
    </cffunction>

    <!--- Called remotely. --->
    <!--- A second comment line. --->
    <cffunction name="api" access="remote" returnformat="json">
        <cfreturn {}>
    </cffunction>

    <!--- Formats a value. --->
    <cffunction name="format" access="public" returntype="string" output="false">
        <cfargument name="value" type="string" required="true">
        <cfargument name="width" type="numeric" default="80">
        <cfreturn left(arguments.value, arguments.width)>
    </cffunction>

    <cffunction name="helper" access=private output="false">
        <cfargument name="x">
        <cfreturn x>
    </cffunction>

</cfcomponent>
