//
item.FirstEntityName = recordStore.findAllMatching('FirstEntityName', {ParentKey: item.ParentKey}, {'orderby': 'SortColumnName desc'});
entities = recordStore.findAllMatching('SecondEntity', {OtherEntityID: otherEntityIDs, Active: true}, {'orderby': 'SortColumnB desc', 'cache': false});
opts = recordStore.findAllMatching('FirstEntityName', 'a very long string argument that pushes the line', {'orderby': 'SortColumnName desc'});
pairs = recordStore.findAllMatching('FirstEntityName', ['FirstEntityRowID', 'ParentKey', 'Active'], {'orderby': 'SortColumnName desc'});
lists = recordStore.findAllMatching('FirstEntityName', ['FirstEntityRowID', 'ParentKey', 'Active'], ['SortColumnName desc', 'ParentKey asc']);
both = runInParallel(someValue, function() { return doTheFirstThing(someValue); }, function() { return doTheSecondThing(); });
