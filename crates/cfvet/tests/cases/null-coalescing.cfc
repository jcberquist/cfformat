component {
    function render(required struct attributes) {
        local.description = attributes.description ?? "";
        var title = local.title ?? "untitled";
        local.author = attributes.author ?? local.author ?? "anonymous";
        return local.description & title & local.author;
    }
}
