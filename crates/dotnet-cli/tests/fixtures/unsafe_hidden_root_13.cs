using System.Runtime.CompilerServices;
class Program {
    static int Main() {
        byte[] blob = new byte[1024];
        object hidden = new object();
        
        // Write object ref into byte array (GC doesn't know it's here)
        Unsafe.WriteUnaligned(ref blob[0], hidden);
        
        hidden = null;
        
        // Trigger intensive GC
        for(int i=0; i<1000; i++) { var _ = new byte[100]; }
        
        // Read it back
        object recovered = Unsafe.ReadUnaligned<object>(ref blob[0]);
        
        // Access it (crash if collected)
        return recovered.ToString().Length;
    }
}
