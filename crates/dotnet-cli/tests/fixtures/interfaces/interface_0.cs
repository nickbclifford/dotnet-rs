interface IValue {
    int GetValue();
}
class Value : IValue {
    public int GetValue() => 42;
}
public class Program {
    public static int Main() {
        IValue val = new Value();
        if (val.GetValue() != 42) return 1;
        return 0;
    }
}
