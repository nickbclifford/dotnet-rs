interface ISetter<in T> {
    void SetValue(T t);
}

class ObjectSetter : ISetter<object> {
    public int Result = 0;
    public void SetValue(object o) {
        if (o is string) Result = 42;
    }
}

public class Program {
    public static int Main() {
        ObjectSetter setter = new ObjectSetter();
        ISetter<string> s = setter;
        s.SetValue("42");
        return setter.Result;
    }
}
