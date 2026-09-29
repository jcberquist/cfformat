component {

    remote function api() {}

    function after( rc ) {}

    // Runs before every action.
    function before( rc ) {}

    function Init() {
        return this;
    }

    function list( rc ) {}

    private function helper() {
        return 1;
    }

}
