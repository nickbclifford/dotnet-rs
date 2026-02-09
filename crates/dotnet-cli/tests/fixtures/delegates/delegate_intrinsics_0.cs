using System;

public class Program {
    delegate int SimpleOp(int x);
    
    public static int Double(int x) => x * 2;
    public int InstanceDouble(int x) => x * 2;
    
    public static int Main() {
        SimpleOp d1 = Double;
        SimpleOp d2 = Double;
        SimpleOp d3 = d1;
        
        // Test Target (static method should have null target)
        if (d1.Target != null) return 1;
        
        Program p = new Program();
        SimpleOp d4 = p.InstanceDouble;
        // Test Target (instance method should have target object)
        if (d4.Target != p) return 2;
        
        // Test Method
        if (d1.Method == null) return 3;
        if (d1.Method.Name != "Double") return 4;
        
        // Test Equals
        if (!d1.Equals(d2)) return 5;
        if (!d1.Equals(d3)) return 6;
        if (d1.Equals(d4)) return 7;
        
        // Test GetHashCode
        if (d1.GetHashCode() != d2.GetHashCode()) return 8;
        
        // Multicast
        SimpleOp m1 = d1;
        m1 += d4;
        SimpleOp m2 = d2;
        m2 += p.InstanceDouble;
        
        if (!m1.Equals(m2)) return 10;
        
        m2 += Double;
        if (m1.Equals(m2)) return 11;

        return 0;
    }
}
