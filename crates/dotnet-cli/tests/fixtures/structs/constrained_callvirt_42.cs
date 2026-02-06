using System;

struct MyStruct {
    public int Value;
    public MyStruct(int v) { Value = v; }
    public override int GetHashCode() => Value;
}

struct SimpleStruct {
    public int Value;
    public override int GetHashCode() => Value;
}

class Program {
    static int CallGetHashCode<T>(T value) {
        return value.GetHashCode();
    }

    static int Main() {
        var s = new MyStruct(42);
        if (CallGetHashCode(s) != 42) return 1;

        var ss = new SimpleStruct();
        ss.Value = 17;
        if (CallGetHashCode(ss) != 17) return 2;

        return 42;
    }
}
