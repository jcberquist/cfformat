// A comment between a keyword and its first attribute ends the keyword's
// line and breaks the attributes, as a comment between two attributes does.

component
	// before the first attribute
	extends="Base"
{
	property
		// before the first attribute
		name="x";

	property name="y"
		// between two attributes
		type="string";

	property /* a block comment */ name="z";

	param
		// before the first attribute
		name="p" default="1";

	param name="q" // on the keyword's line
		default="2";

	http
		// before the first attribute
		url="x";

	http // on the tag's line
		url="y" method="get";

	lock
		// before the first attribute
		name="x" timeout="1"
	{
		x = 1;
	}

	function f() output="false"
		// between two attributes
		returntype="string"
	{
		return 1;
	}
}
