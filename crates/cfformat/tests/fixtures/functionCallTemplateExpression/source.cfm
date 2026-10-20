<cfoutput>
  #doStuff('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever_this_is_really_long_to_handle')#
  <cfset x = doStuff('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever_this_is_really_long_to_handle')>
  #doStuff('a', 'b')# stays on its line, and so do #now()# and #x#
  #x#more and text #x# text and #a##b# glued
  #doStuff('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever_this_is_really_long_to_handle')#more
  text #doStuff('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever_this_is_really_long_to_handle')# text
  #variables.service.doStuff('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever')#
  #first & " " & second & " " & third & " " & fourth & " " & fifth & " " & sixth & " " & seventh & " " & eighth & " " & ninth#
  #isDefined("url.something_long_enough") ? url.something_long_enough_value : variables.default_fallback_value_name_which_is_longer#
  #doStuff('something', // a comment
    'anothervalue')#
  #doStuff('a') // a comment ends the line, so the delimiters stay where they are
  #
</cfoutput>
<cfoutput>#doStuff('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever_this_is_really_long_to_handle')#</cfoutput>
<p><cfoutput>#doStuff('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever_this_is_really_long_to_handle')#</cfoutput></p>
<cfoutput>#a#</cfoutput>
<cfoutput>
  #first & second & third# fits, and so do #a eq b# and #a ?: b#
  #someLongConditionVariableName and anotherLongConditionVariableName and yetAnotherLongConditionName or stillAnotherConditionName#
  #someLongVariableNameForTheComparison eq anotherLongVariableNameForTheComparison_which_is_longer_still_than_the_first#
  #someLongVariableNameForTheDefaultValue ?: anotherLongVariableNameForTheFallbackValueHere_which_is_the_elvis_operand#
  #isDefined("url.something_long_enough") ? doStuff("something", "anothervalue", "third") : doOtherStuff("something", "anothervalue")#
  #doStuff(isDefined("url.something_long_enough") ? url.something_long_enough_value : variables.default_fallback_value_name, "another")#
  #doStuff("something", "anothervalue", "thirdvalue", "fourth_thingy", "fiftharg") & barbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbar#
  #doStuff("something", "anothervalue", "thirdvalue", "fourth_thingy", "fiftharg", "sixth_whatever_this_is_really_long_to_handle") & bar#
  #variables.someService.getSomething("a long argument", "another long argument", "and a third one that is long").someProperty#
  #duplicate(prc.widgets).filter(function (w) { return w.active; }).recordcount#
</cfoutput>
