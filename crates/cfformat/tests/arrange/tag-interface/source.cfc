<cfinterface displayname="Repository">
    <cffunction name="save" access="public" returntype="void">
        <cfargument name="entity" type="any" required="true">
    </cffunction>
    <cffunction name="find" access="public" returntype="any">
        <cfargument name="id" type="numeric" required="true">
    </cffunction>
    <cffunction name="delete" access="public" returntype="void">
        <cfargument name="id" type="numeric" required="true">
    </cffunction>
</cfinterface>
