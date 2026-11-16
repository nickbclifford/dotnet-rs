using System; using System.Reflection; using System.Reflection.Emit;
class Program {
 public static int Add(int a,int b) => a+b;
 static int Main() {
  try { var m=new DynamicMethod("Middle",typeof(int),Type.EmptyTypes); var il=m.GetILGenerator();
   var s=SignatureHelper.GetMethodSigHelper(CallingConventions.Standard,typeof(int));
   s.AddArgument(typeof(int)); s.AddSentinel(); s.AddArgument(typeof(int));
   il.Emit(OpCodes.Ldc_I4,20); il.Emit(OpCodes.Ldc_I4,22); il.Emit(OpCodes.Ldftn,typeof(Program).GetMethod(nameof(Add))!);
   il.Emit(OpCodes.Calli,s); il.Emit(OpCodes.Ret); Console.WriteLine(((Func<int>)m.CreateDelegate(typeof(Func<int>)))());
  } catch(Exception e) { Console.WriteLine(e.GetType().Name); } return 42;
 }
}
