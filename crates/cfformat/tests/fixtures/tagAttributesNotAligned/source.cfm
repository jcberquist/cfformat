<cf_render_input name="username" id="username" label="Username to impersonate" value="#request.params.username.encodeforHTML()#" maxlength="7" required autofocus>

<input type="text" name="username" id="username" required placeholder="Enter the username to impersonate" maxlength="7" class="form-control input-lg">

<cf_widget title="Hello" count=#n# description="A widget with a description long enough that its attributes cannot stay on one line">

<cf_render_input
	name     ="username"
	id       ="username"
	label    ="Username to impersonate"
	value    ="#request.params.username.encodeforHTML()#"
	maxlength="7"
	required
	autofocus
>

<cf_render_input
	name      = "username"
	id        = "username"
	label     = "Username to impersonate"
	value     = "#request.params.username.encodeforHTML()#"
	maxlength = "7"
	required
	autofocus
>

<cf_mixed a="1" b = "2">

<cfquery name="q" datasource="ds">select 1</cfquery>

<div class="container-fluid wide-layout" title="the main area" <cfif isActive>data-state="active"</cfif> id="main-content-area" role="main">x</div>

<cfscript>
property name="firstPropertyName" type="string" default="a long default value for the first" hint="the first property of the component";
property name="second" type="numeric" default="a long default value for the second property" hint="the second property of the component";

http url="x" method="get" result="r";
http url="https://example.com/a/long/path/to/an/endpoint/that/does/not/fit/on/one/line" method="get" result="response";
</cfscript>
