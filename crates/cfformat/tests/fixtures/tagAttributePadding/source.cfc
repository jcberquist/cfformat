<cfparam name="url.page" default="1">
<cfquery name="q" datasource = "#dsn#">select 1</cfquery>
<cf_widget title="Hello" count=#n#>
<ui:card title='Card'/>
<div class="c" id='main'>
<a href="x.cfm?page=#url.page#" title='Next'>Next</a>
</div>
<cfset x = 1>
<cfset y=x+1>
<cfscript>
param name="p" default="1";
cfhttp(url="x", method="get");
</cfscript>
