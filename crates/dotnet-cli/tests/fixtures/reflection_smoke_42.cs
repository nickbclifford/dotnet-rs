using System;

public class Program {
    public static int Main() {
        Type t1 = typeof(int);
        Type t2 = typeof(int);
        
        if (!ReferenceEquals(t1, t2)) return 1;
        if (t1.Name != "Int32") return 2;
        
        Type t3 = typeof(string);
        if (ReferenceEquals(t1, t3)) return 3;
        
        return 42;
    }
}
