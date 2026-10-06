// add the flag -DEP464_INT=int
EP464_INT bit_array_size(EP464_INT k);
EP464_INT work(EP464_INT k, char *all_mem);
char bss[1 << 17];
EP464_INT entry_point(EP464_INT k) {
  EP464_INT k_size = sizeof(k);
  EP464_INT bit_array_sz = bit_array_size(k);
  EP464_INT all_mem_size = 3 * bit_array_sz + ((k + 1) * 2 * k_size);
  if (all_mem_size > (1 << 17))
    return -1;
  EP464_INT ret = work(k, bss);
  return ret;
}
