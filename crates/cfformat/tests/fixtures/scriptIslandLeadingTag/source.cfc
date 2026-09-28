<cfoutput>
<div>
<script>
<cfif local.debug>
console.log("#local.message#");
</cfif>
start();
</script>
<style>#fileRead(expandPath("./report.css"))#</style>
<script type="text/javascript"><cfinclude template="widget.js"></script>
<script>
<!--
legacy();
-->
</script>
<style>
<!--- a tag comment first --->
body { margin: 0; }
</style>
</div>
</cfoutput>
