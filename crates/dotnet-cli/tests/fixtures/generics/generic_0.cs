class Box<T> {
    public T Value;
}
public class Program {
    public static int Main() {
        Box<int> b = new Box<int>();
        b.Value = 42;
        if (b.Value != 42) return 1;
        
        Box<string> s = new Box<string>();
        s.Value = "Hello";
        if (s.Value != "Hello") return 2;
        
        return 0;
    }
}
