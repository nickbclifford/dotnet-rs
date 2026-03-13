using System;

interface IGetter<T> {
    T GetValue();
}

class MultiGetter : IGetter<string>, IGetter<int> {
    string IGetter<string>.GetValue() => "42";
    int IGetter<int>.GetValue() => 42;
}

public class Program {
    public static int Main() {
        IGetter<string> s = new MultiGetter();
        IGetter<int> i = new MultiGetter();
        if (s.GetValue() == "42" && i.GetValue() == 42) return 42;
        return 0;
    }
}
