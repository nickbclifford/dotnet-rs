// File: basic/switch_statement_42.cs
// Purpose: Test switch instruction
// Expected: Returns 42 on success

using System;

public class Program {
    public static int Main() {
        if (TestSwitch(0) != 10) return 1;
        if (TestSwitch(1) != 20) return 2;
        if (TestSwitch(2) != 30) return 3;
        if (TestSwitch(3) != 40) return 4;
        if (TestSwitch(-1) != 0) return 5;
        if (TestSwitch(4) != 0) return 6;

        return 42;
    }

    static int TestSwitch(int x) {
        switch (x) {
            case 0: return 10;
            case 1: return 20;
            case 2: return 30;
            case 3: return 40;
            default: return 0;
        }
    }
}
