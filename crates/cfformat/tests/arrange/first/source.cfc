component {

    private function helper() {
        return 1;
    }

    function after( rc ) {}

    remote function api() {}

    function Init() {
        return this;
    }

    // Runs before every action.
    function before( rc ) {}

    function list( rc ) {}

}
