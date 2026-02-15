using System;

public struct GenericStruct<T> {
    public T Value;
    public int Extra;
}

public class Program {
    public static void Increment(ref int x) {
        x++;
    }

    public static void SetValue<T>(ref T target, T val) {
        target = val;
    }

    public static int Main() {
        GenericStruct<int> s1 = new GenericStruct<int>();
        s1.Value = 10;
        s1.Extra = 20;

        Increment(ref s1.Value);
        Increment(ref s1.Extra);

        if (s1.Value != 11 || s1.Extra != 21) return 1;

        GenericStruct<string> s2 = new GenericStruct<string>();
        s2.Value = "hello";
        SetValue(ref s2.Value, "world");

        if (s2.Value != "world") return 2;

        // Nested generic structs on stack
        GenericStruct<GenericStruct<int>> s3 = new GenericStruct<GenericStruct<int>>();
        s3.Value.Value = 100;
        Increment(ref s3.Value.Value);

        if (s3.Value.Value != 101) return 3;

        return 42;
    }
}
