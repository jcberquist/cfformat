component {

    function Init() {
        return this;
    }

    remote function api() {}

    function after( rc ) {}

    // Runs before every action.
    function before( rc ) {}

    function list( rc ) {}

    private function helper() {
        return 1;
    }

}
