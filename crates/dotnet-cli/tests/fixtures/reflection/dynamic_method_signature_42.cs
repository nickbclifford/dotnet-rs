using System;
using System.Reflection;
using System.Reflection.Emit;
class Program {
 public static int Twice(int x) => x * 2;
 static int Main() {
 var m=new DynamicMethod("Call", typeof(int), Type.EmptyTypes); var il=m.GetILGenerator();
 var sig=SignatureHelper.GetMethodSigHelper(CallingConventions.Standard, typeof(int)); sig.AddArgument(typeof(int));
 Console.WriteLine(Convert.ToHexString(sig.GetSignature()));
 il.Emit(OpCodes.Ldc_I4, 21); il.Emit(OpCodes.Ldftn, typeof(Program).GetMethod(nameof(Twice))!); il.Emit(OpCodes.Calli, sig); il.Emit(OpCodes.Ret);
 int r=((Func<int>)m.CreateDelegate(typeof(Func<int>)))(); Console.WriteLine(r); return r;
 }
}
