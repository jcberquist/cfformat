component {

    public function Init() {
        return this;
    }

    function epsilon() access="remote" {}

    remote string function gamma() {}

    public static function Apple() {}

    function beta() {}

    function alpha() access="package" {}

    package function delta() {}

    private function eta() access="PRIVATE" {}

    static private function omega() {}

    private function zeta() {
        return 1;
    }

}
