using System;

struct MyStruct {
    public int Value;
    public MyStruct(int v) { Value = v; }
    public override string ToString() => Value.ToString();
}

struct SimpleStruct {
    public int Value;
}

class Program {
    static string CallToString<T>(T value) {
        return value.ToString();
    }
    
    static int Main() {
        var s = new MyStruct(42);
        if (CallToString(s) != "42") return 1;
        
        var ss = new SimpleStruct();
        if (CallToString(ss) != "SimpleStruct") return 2;
        
        string str = "42";
        if (CallToString(str) != "42") return 3;
        
        return 42;
    }
}
