using System;

public class Program {
    public static int Main() {
        int result = 0;
        Action<int> a1 = x => result += x;
        Action<int> a2 = x => result += x * 2;
        Action<int> combo = a1 + a2;
        
        combo(10); // result = 10 + 20 = 30
        if (result != 30) return 1;
        
        Action<int> removed = combo - a1;
        result = 0;
        removed(10); // result = 20
        if (result != 20) return 2;
        
        Action<int> removed2 = combo - a2;
        result = 0;
        removed2(10); // result = 10
        if (result != 10) return 3;
        
        Action<int> empty = combo - combo;
        if (empty != null) return 4;
        
        return 42;
    }
}
