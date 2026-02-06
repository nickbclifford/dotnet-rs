using System;

public class Program {
    public static int Main() {
        if (TestAddOvf() != 0) return 1;
        if (TestSubOvf() != 0) return 2;
        if (TestMulOvf() != 0) return 3;
        if (TestAddOvfUn() != 0) return 4;
        if (TestSubOvfUn() != 0) return 5;
        if (TestMulOvfUn() != 0) return 6;
        
        return 42;
    }

    static int TestAddOvf() {
        try {
            int a = int.MaxValue;
            int b = 1;
            int c = checked(a + b);
            return -1;
        } catch (OverflowException) {
            return 0;
        }
    }

    static int TestSubOvf() {
        try {
            int a = int.MinValue;
            int b = 1;
            int c = checked(a - b);
            return -1;
        } catch (OverflowException) {
            return 0;
        }
    }

    static int TestMulOvf() {
        try {
            int a = int.MaxValue / 2 + 1;
            int b = 2;
            int c = checked(a * b);
            return -1;
        } catch (OverflowException) {
            return 0;
        }
    }

    static int TestAddOvfUn() {
        try {
            uint a = uint.MaxValue;
            uint b = 1;
            uint c = checked(a + b);
            return -1;
        } catch (OverflowException) {
            return 0;
        }
    }

    static int TestSubOvfUn() {
        try {
            uint a = 0;
            uint b = 1;
            uint c = checked(a - b);
            return -1;
        } catch (OverflowException) {
            return 0;
        }
    }

    static int TestMulOvfUn() {
        try {
            uint a = uint.MaxValue / 2 + 1;
            uint b = 2;
            uint c = checked(a * b);
            return -1;
        } catch (OverflowException) {
            return 0;
        }
    }
}
