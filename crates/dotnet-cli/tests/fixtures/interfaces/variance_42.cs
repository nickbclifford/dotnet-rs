interface IGetter<out T> {
    T GetValue();
}

class StringGetter : IGetter<string> {
    public string GetValue() => "42";
}

public class Program {
    public static int Main() {
        IGetter<object> getter = new StringGetter();
        object val = getter.GetValue();
        if (val.ToString() == "42") return 42;
        return 0;
    }
}
