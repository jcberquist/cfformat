<!--- Outside <cfoutput>: HTML text, attribute values, islands. --->
<p>Hello #getName()#</p>
<a href="#buildLink( 'x' )#" onclick="#showDetail("x")#" style="width: #w()#">y</a>
<script>var a = "#serializeJSON( data )#";</script>
<p>#rc.user.getName()# #a.b( 1 ).c[ 2 ]()#</p>
<cfsavecontent variable="s"><p>#saved()#</p></cfsavecontent>
<cfif true><p>#inIf()#</p></cfif>
<!--- Not calls, not output, or live. --->
<p style="color: #fff; background: #000">#name# #a.b# ## <a href="#top">x</a></p>
<!-- #htmlComment()# -->
<cfset y = "#live()#">
<cfoutput><p>#live()#</p></cfoutput>
<cfmail to="x" from="y" subject="z">#live()#</cfmail>
<cfquery name="q">select #live()#</cfquery>
<!--- A <cfoutput> opened in a <style> runs past its </style>. --->
<style>
<cfoutput>
.a { color: #c# }
</style>
<p>#live()#</p>
</cfoutput>
<cffunction name="on" output="true"><p>#live()#</p></cffunction>
<cffunction name="yes" output="yes"><p>#live()#</p></cffunction>
<cffunction name="off" output="false"><p>#off()#</p></cffunction>
<cffunction name="absent"><p>#absent()#</p></cffunction>
<p>#ignored()#</p> <!--- cfvet-ignore --->
